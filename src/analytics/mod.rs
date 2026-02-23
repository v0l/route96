use anyhow::Error;
use axum::{extract::Request, response::Response};
use log::warn;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower::{Layer, Service};

pub mod plausible;

pub trait Analytics {
    fn track(&self, req: &Request) -> Result<(), Error>;
}

#[derive(Clone)]
pub struct AnalyticsLayer {
    inner: Arc<dyn Analytics + Sync + Send>,
}

impl AnalyticsLayer {
    pub fn new<T>(inner: T) -> Self
    where
        T: Analytics + Send + Sync + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }
}

impl<S> Layer<S> for AnalyticsLayer {
    type Service = AnalyticsMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AnalyticsMiddleware {
            inner,
            analytics: self.inner.clone(),
        }
    }
}

#[derive(Clone)]
pub struct AnalyticsMiddleware<S> {
    inner: S,
    analytics: Arc<dyn Analytics + Sync + Send>,
}

impl<S> Service<Request> for AnalyticsMiddleware<S>
where
    S: Service<Request, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let analytics = self.analytics.clone();

        if let Err(e) = analytics.track(&req) {
            warn!("Failed to track! {}", e);
        }

        let future = self.inner.call(req);
        Box::pin(future)
    }
}
