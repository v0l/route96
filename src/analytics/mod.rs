use anyhow::Error;
use log::warn;
use rocket::fairing::{Fairing, Info, Kind};
use rocket::{Data, Request};

pub mod plausible;

pub trait Analytics {
    fn track(&self, req: &Request) -> Result<(), Error>;
}

pub struct AnalyticsFairing {
    inner: Box<dyn Analytics + Sync + Send>,
}

impl AnalyticsFairing {
    pub fn new<T>(inner: T) -> Self
    where
        T: Analytics + Send + Sync + 'static,
    {
        Self {
            inner: Box::new(inner),
        }
    }
}

#[rocket::async_trait]
impl Fairing for AnalyticsFairing {
    fn info(&self) -> Info {
        Info {
            name: "Analytics",
            kind: Kind::Request,
        }
    }

    async fn on_request(&self, req: &mut Request<'_>, _data: &mut Data<'_>) {
        if let Err(e) = self.inner.track(req) {
            warn!("Failed to track! {}", e);
        }
    }
}
