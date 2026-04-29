use axum::extract::Request;
use axum::http::HeaderValue;
use axum::response::Response;
use log::warn;
use std::future::Future;
use std::pin::Pin;
use tower::{Layer, Service};

/// Layer that logs non-2xx responses with the request method, path, status
/// code, and the `x-reason` header (if present).
#[derive(Clone)]
pub struct ResponseLogLayer;

impl<S> Layer<S> for ResponseLogLayer {
    type Service = ResponseLogService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ResponseLogService { inner }
    }
}

#[derive(Clone)]
pub struct ResponseLogService<S> {
    inner: S,
}

impl<S> Service<Request> for ResponseLogService<S>
where
    S: Service<Request, Response = Response> + Send + Clone + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let method = req.method().clone();
        let path = req.uri().path().to_owned();

        // Clone the inner service so we can move it into the async block.
        // This is the standard pattern for axum middleware.
        let mut inner = self.inner.clone();

        Box::pin(async move {
            let response = inner.call(req).await?;

            let status = response.status();
            if status.as_u16() >= 400 {
                let reason = response
                    .headers()
                    .get("x-reason")
                    .and_then(|v: &HeaderValue| v.to_str().ok())
                    .unwrap_or("");

                if reason.is_empty() {
                    warn!("{} {} -> {}", method, path, status);
                } else {
                    warn!("{} {} -> {} ({})", method, path, status, reason);
                }
            }

            Ok(response)
        })
    }
}
