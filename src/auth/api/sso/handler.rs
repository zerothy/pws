use axum::{ 
    extract::{State, Json},
};
use hyper::{Body, Response, StatusCode};
use garde::Unvalidated;
use serde_json::json;

use crate::{
    auth::{Auth, SsoCallbackRequest},
    startup::AppState
};
use super::client::CasClient;

#[tracing::instrument(skip(auth))]
pub async fn handle_callback(
    auth: Auth,
    State(AppState { .. }): State<AppState>,
    Json(req): Json<Unvalidated<SsoCallbackRequest>>,
) -> Response<Body> {
    let SsoCallbackRequest { ticket, service_url } = match req.validate(&()) {
        Ok(validated) => validated.into_inner(),
        Err(err) => {
            let body = serde_json::to_string(&json!({ "error": err.to_string() }))
                .unwrap();
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .unwrap();
        }
    };

    let cas_server_url = "https://sso.ui.ac.id/cas2/";
    let client = CasClient::new(service_url.clone(), cas_server_url, None);


    match client.verify_ticket(&ticket).await {
        Ok(profile) => {
            let body = serde_json::to_string(&json!({
                "username": profile.username,
                "attributes": profile.attributes,
            }))
            .unwrap();

            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .unwrap()
        }
        Err(err) => {
            eprintln!("CAS verification failed: {:?}", err);
            let body = serde_json::to_string(&json!({ "error": "Invalid ticket" }))
                .unwrap();

            Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .unwrap()
        }
    }
}

