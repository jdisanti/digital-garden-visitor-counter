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

//! Request information extraction and bot detection.

use isbot::Bots;
use lambda_http::{request::RequestContext, Request, RequestExt};
use once_cell::sync::Lazy;
use std::{error::Error as StdError, fmt};

/// Initialize the bot checker once and reuse it for every request.
static BOT_CHECKER: Lazy<Bots> = Lazy::new(Bots::default);

/// An error extracting request information from the request.
#[derive(Debug)]
pub enum RequestInfoError {
    /// The request is missing a user agent.
    MissingUserAgent,
    /// The request is missing a source IP.
    MissingSourceIp,
    /// The request looks like a bot, and thus, we should reject it.
    LooksLikeABot,
}

impl StdError for RequestInfoError {}

impl fmt::Display for RequestInfoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MissingUserAgent => "request has no user agent",
            Self::MissingSourceIp => "request has no source IP",
            Self::LooksLikeABot => "request looks like a bot",
        })
    }
}

/// Information extracted from a request that is needed to
/// semi-uniquely identify a visitor to avoid inflation of the counter.
pub struct RequestInfo {
    /// User agent header value.
    pub user_agent: String,
    /// Source IP address, which can be in IPv4 or IPv6 format.
    pub source_ip: String,
}

impl TryFrom<&Request> for RequestInfo {
    type Error = RequestInfoError;

    /// Try to extract request information from the request, and return
    /// an error for any request that looks like its from a bot.
    fn try_from(value: &Request) -> Result<Self, Self::Error> {
        let RequestContext::ApiGatewayV2(context) = value
            .request_context_ref()
            .expect("missing request context");
        let user_agent = context
            .http
            .user_agent
            .as_ref()
            .ok_or(RequestInfoError::MissingUserAgent)?;

        // Reject bots that are identified by the user agent.
        if BOT_CHECKER.is_bot(user_agent) {
            return Err(RequestInfoError::LooksLikeABot);
        }

        let source_ip = context
            .http
            .source_ip
            .as_ref()
            .ok_or(RequestInfoError::MissingSourceIp)?;
        Ok(RequestInfo {
            user_agent: user_agent.into(),
            source_ip: source_ip.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use lambda_http::aws_lambda_events::apigw::{
        ApiGatewayV2httpRequestContext, ApiGatewayV2httpRequestContextHttpDescription,
    };

    use super::*;

    fn request(ua: Option<&str>, ip: Option<&str>) -> lambda_http::Request {
        http::Request::builder()
            .method("GET")
            .uri("/some-url")
            .extension(RequestContext::ApiGatewayV2(
                ApiGatewayV2httpRequestContext {
                    http: ApiGatewayV2httpRequestContextHttpDescription {
                        user_agent: ua.map(String::from),
                        source_ip: ip.map(String::from),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ))
            .body(lambda_http::Body::Empty)
            .unwrap()
    }

    #[test]
    fn valid_request() {
        let request = request(Some("foo bar baz"), Some("127.0.0.1"));
        let info = RequestInfo::try_from(&request).unwrap();
        assert_eq!("foo bar baz", info.user_agent);
        assert_eq!("127.0.0.1", info.source_ip);
    }

    #[test]
    fn missing_user_agent() {
        let request = request(None, Some("127.0.0.1"));
        assert!(matches!(
            RequestInfo::try_from(&request),
            Err(RequestInfoError::MissingUserAgent)
        ));
    }

    #[test]
    fn missing_source_ip() {
        let request = request(Some("foo bar baz"), None);
        assert!(matches!(
            RequestInfo::try_from(&request),
            Err(RequestInfoError::MissingSourceIp)
        ));
    }

    #[test]
    fn bot() {
        let request = request(Some("irbot"), Some("127.0.0.1"));
        assert!(matches!(
            RequestInfo::try_from(&request),
            Err(RequestInfoError::LooksLikeABot)
        ));
    }
}
