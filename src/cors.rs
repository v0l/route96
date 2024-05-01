use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::{Header, Method, Status};
use rocket::{Request, Response};
use std::io::Cursor;

pub struct CORS;

#[rocket::async_trait]
impl Fairing for CORS {
    fn info(&self) -> Info {
        Info {
            name: "CORS headers",
            kind: Kind::Response,
        }
    }

    async fn on_response<'r>(&self, req: &'r Request<'_>, response: &mut Response<'r>) {
        response.set_header(Header::new("Access-Control-Allow-Origin", "*"));
        response.set_header(Header::new(
            "Access-Control-Allow-Methods",
            "PUT, GET, HEAD, DELETE, OPTIONS",
        ));
        response.set_header(Header::new("Access-Control-Allow-Headers", "*"));
        response.set_header(Header::new("Access-Control-Allow-Credentials", "true"));

        // force status 200 for options requests
        if req.method() == Method::Options {
            response.set_status(Status::Ok);
            response.set_sized_body(None, Cursor::new(""))
        }
    }
}
