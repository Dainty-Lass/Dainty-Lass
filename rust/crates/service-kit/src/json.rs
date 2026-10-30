// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Serialize, Serializer};

pub const JSON_UTF8: &str = "application/json; charset=utf-8";

#[derive(Debug, Clone, Copy)]
pub struct Json<T>(pub T);

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        match serde_json::to_value(self.0) {
            Ok(val) => {
                let body = crate::jsjson::stringify(&val);
                ([(header::CONTENT_TYPE, JSON_UTF8)], body).into_response()
            }
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
}

pub fn js_number<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    if !value.is_finite() {
        return serializer.serialize_none();
    }
    if *value == 0.0 {
        return serializer.serialize_i64(0);
    }
    if value.fract() == 0.0 && value.abs() < 1e21 {
        if (*value >= i64::MIN as f64) && (*value <= i64::MAX as f64) {
            let i = *value as i64;
            if (i as f64) == *value {
                return serializer.serialize_i64(i);
            }
        }
        return serializer.serialize_i128(*value as i128);
    }
    serializer.serialize_f64(*value)
}

pub fn js_number_opt<S: Serializer>(value: &Option<f64>, serializer: S) -> Result<S::Ok, S::Error> {
    match value {
        Some(v) => js_number(v, serializer),
        None => serializer.serialize_none(),
    }
}

pub fn not_found(_request: &Request) -> Response {
    StatusCode::NOT_FOUND.into_response()
}
