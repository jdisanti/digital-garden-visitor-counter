// Digital garden visitor counter
// A simple visitor counter for digital gardens that runs as an AWS Lambda function.
// Copyright (C) 2023 John DiSanti.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

use digital_garden_visitor_counter::{
    counter::render_separated_number,
    request_info::{RequestInfo, RequestInfoError},
    store::{Store, Visitor},
};
use lambda_http::{run, service_fn, Body, Error, Request, RequestExt, Response};
use std::{sync::Arc, time::SystemTime};

/// Configuration for the Lambda, set by environment variables.
struct Config {
    /// DynamoDB table name, set by the `GHC_TABLE_NAME` environment variable.
    table_name: String,
    /// Minimum width of the rendered image in number of characters.
    min_width: usize,
    /// Allowed counter names, set by the `GHC_ALLOWED_NAMES` environment variable (comma-delimited).
    allowed_names: Vec<String>,
}

impl Config {
    fn from_env() -> Self {
        Self {
            table_name: std::env::var("DGVC_TABLE_NAME")
                .ok()
                .unwrap_or_else(|| "garden-hit-counter".into()),
            min_width: std::env::var("DGVC_MIN_WIDTH")
                .ok()
                .map(|n| n.parse().unwrap())
                .unwrap_or(5),
            allowed_names: std::env::var("DGVC_ALLOWED_NAMES")
                .ok()
                .map(|s| s.split(',').map(String::from).collect())
                .unwrap_or_else(|| vec!["default".into()]),
        }
    }
}

fn not_found() -> Response<Body> {
    Response::builder()
        .status(404)
        .body(Body::Empty)
        .expect("valid response")
}

async fn function_handler(
    config: Arc<Config>,
    store: Arc<Store>,
    event: Request,
) -> Result<Response<Body>, Error> {
    // Don't respond to non-root requests, such as `/favicon.ico`.
    if event.uri().path() != "/" {
        return Ok(not_found());
    }

    // Extract some information from the request.
    let request_info = match RequestInfo::try_from(&event) {
        Ok(info) => info,
        // Quickly reject bots to avoid inflating the counter and reduce costs.
        Err(RequestInfoError::LooksLikeABot) => {
            return Ok(not_found());
        }
        Err(err) => return Err(err.into()),
    };

    // Create a semi-unique hash of the visitor's IP and user agent.
    let visitor = Visitor::from(&request_info);

    // Get the name of the counter to increment from query parameters.
    let count_name = event
        .query_string_parameters_ref()
        .and_then(|params| params.first("name"))
        .unwrap_or("default");

    // Security: Reject any names that are not allow listed.
    if !config.allowed_names.iter().any(|name| name == count_name) {
        return Ok(not_found());
    }

    // Privacy: This only temporarily stores a 32-bit hash of the visitor's IP and user agent
    // so that we can roughly track uniqueness without storing any identifying information.
    let count = store
        .maybe_increment_visitors(visitor, count_name, SystemTime::now())
        .await?;

    // Render the counter to an in-memory PNG.
    let render = render_separated_number(count, config.min_width);
    let png_bytes = render.to_png_bytes()?;

    Ok(Response::builder()
        .status(200)
        .header("cache-control", "no-cache")
        .header("content-type", "image/png")
        .header("content-length", png_bytes.len())
        .header("x-count-name", count_name)
        .header("x-count", count)
        .header("x-tag", visitor.tag)
        .body(Body::Binary(png_bytes))
        .expect("valid response"))
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .without_time()
        .init();

    let config = Arc::new(Config::from_env());
    let store = Arc::new(Store::new(config.table_name.clone()).await);

    run(service_fn(move |event| {
        function_handler(config.clone(), store.clone(), event)
    }))
    .await
}
