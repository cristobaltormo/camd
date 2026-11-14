use serde_json::{Map, Value, json};

use super::{ApiError, BodyJson, Ctx};

pub fn json_invalid(body: &[u8], error: &serde_json::Error) -> ApiError {
    let text = String::from_utf8_lossy(body);
    let message = error.to_string();
    let (position, reason) = if error.is_eof() {
        let last = text.trim_end().chars().last();
        let reason =
            if message.contains("parsing an object") && matches!(last, Some('{') | Some(',')) {
                "Expecting property name enclosed in double quotes"
            } else {
                "Expecting value"
            };
        (text.chars().count(), reason)
    } else {
        let mut position = 0usize;
        for (number, line) in text.split('\n').enumerate() {
            if number + 1 == error.line() {
                position += error.column().saturating_sub(1);
                break;
            }
            position += line.chars().count() + 1;
        }
        let reason = if message.starts_with("key must be a string") {
            "Expecting property name enclosed in double quotes"
        } else if message.starts_with("expected `:`") {
            "Expecting ':' delimiter"
        } else if message.starts_with("expected `,`") {
            "Expecting ',' delimiter"
        } else if message.starts_with("trailing") {
            "Extra data"
        } else {
            "Expecting value"
        };
        (position, reason)
    };
    ApiError::Validation(vec![json!({
        "type": "json_invalid",
        "loc": ["body", position],
        "msg": "JSON decode error",
        "input": {},
        "ctx": {"error": reason},
    })])
}

pub fn number(value: f64) -> Value {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn bound(value: f64, integer: bool) -> Value {
    if integer {
        json!(value as i64)
    } else {
        number(value)
    }
}

fn bound_text(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

pub fn parse_int_text(text: &str) -> Option<i64> {
    let clean = text.trim().replace('_', "");
    if let Ok(i) = clean.parse::<i64>() {
        return Some(i);
    }
    match clean.parse::<f64>() {
        Ok(f) if f.is_finite() && f.fract() == 0.0 && f.abs() < 9.0e18 => Some(f as i64),
        _ => None,
    }
}

pub fn parse_float_text(text: &str) -> Option<f64> {
    let clean = text.trim().replace('_', "");
    let lower = clean.to_ascii_lowercase();
    if lower.contains("inf") || lower.contains("nan") {
        return None;
    }
    clean.parse::<f64>().ok()
}

pub fn parse_bool_text(text: &str) -> Option<bool> {
    match text.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "t" | "y" => Some(true),
        "0" | "false" | "no" | "off" | "f" | "n" => Some(false),
        _ => None,
    }
}

#[derive(Default)]
pub struct Check {
    pub errors: Vec<Value>,
}

impl Check {
    pub fn new() -> Check {
        Check::default()
    }

    pub fn error(&mut self, kind: &str, loc: Vec<Value>, msg: impl Into<String>, input: &Value) {
        self.errors
            .push(json!({"type": kind, "loc": loc, "msg": msg.into(), "input": input}));
    }

    pub fn error_ctx(
        &mut self,
        kind: &str,
        loc: Vec<Value>,
        msg: impl Into<String>,
        input: &Value,
        ctx: Value,
    ) {
        self.errors
            .push(json!({"type": kind, "loc": loc, "msg": msg.into(), "input": input, "ctx": ctx}));
    }

    pub fn finish(self) -> Result<(), ApiError> {
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(ApiError::Validation(self.errors))
        }
    }

    fn int_from(&mut self, loc: Vec<Value>, value: &Value) -> Option<i64> {
        let failure = |f: f64| {
            if f.fract() != 0.0 {
                (
                    "int_from_float",
                    "Input should be a valid integer, got a number with a fractional part",
                )
            } else {
                (
                    "int_parsing_size",
                    "Unable to parse input string as an integer, exceeded maximum size",
                )
            }
        };
        let result = match value {
            Value::Bool(b) => Ok(i64::from(*b)),
            Value::Number(n) => match n.as_i64() {
                Some(i) => Ok(i),
                None => {
                    let f = n.as_f64().unwrap_or(f64::NAN);
                    if f.fract() == 0.0 && f.abs() < 9.0e18 {
                        Ok(f as i64)
                    } else {
                        Err(failure(f))
                    }
                }
            },
            Value::String(s) => parse_int_text(s).ok_or((
                "int_parsing",
                "Input should be a valid integer, unable to parse string as an integer",
            )),
            _ => Err(("int_type", "Input should be a valid integer")),
        };
        match result {
            Ok(i) => Some(i),
            Err((kind, msg)) => {
                self.error(kind, loc, msg, value);
                None
            }
        }
    }

    fn float_from(&mut self, loc: Vec<Value>, value: &Value) -> Option<f64> {
        let result = match value {
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Value::Number(n) => n.as_f64(),
            Value::String(s) => parse_float_text(s),
            _ => None,
        };
        if result.is_none() {
            let (kind, msg) = if value.is_string() {
                (
                    "float_parsing",
                    "Input should be a valid number, unable to parse string as a number",
                )
            } else {
                ("float_type", "Input should be a valid number")
            };
            self.error(kind, loc, msg, value);
        }
        result
    }

    fn bool_from(&mut self, loc: Vec<Value>, value: &Value) -> Option<bool> {
        let result = match value {
            Value::Bool(b) => Some(*b),
            Value::Number(n) => match n.as_f64() {
                Some(0.0) => Some(false),
                Some(1.0) => Some(true),
                _ => None,
            },
            Value::String(s) => parse_bool_text(s),
            _ => None,
        };
        if result.is_none() {
            let (kind, msg) = match value {
                Value::String(_) | Value::Number(_) => (
                    "bool_parsing",
                    "Input should be a valid boolean, unable to interpret input",
                ),
                _ => ("bool_type", "Input should be a valid boolean"),
            };
            self.error(kind, loc, msg, value);
        }
        result
    }

    fn range(
        &mut self,
        loc: Vec<Value>,
        value: f64,
        input: &Value,
        low: Option<f64>,
        high: Option<f64>,
        integer: bool,
    ) -> bool {
        if let Some(low) = low.filter(|l| value < *l) {
            self.error_ctx(
                "greater_than_equal",
                loc,
                format!(
                    "Input should be greater than or equal to {}",
                    bound_text(low)
                ),
                input,
                json!({"ge": bound(low, integer)}),
            );
            return false;
        }
        if let Some(high) = high.filter(|h| value > *h) {
            self.error_ctx(
                "less_than_equal",
                loc,
                format!("Input should be less than or equal to {}", bound_text(high)),
                input,
                json!({"le": bound(high, integer)}),
            );
            return false;
        }
        true
    }

    pub fn path_int(&mut self, name: &str, raw: &str) -> Option<i64> {
        self.int_from(vec![json!("path"), json!(name)], &json!(raw))
    }

    pub fn query_int(&mut self, ctx: &Ctx, name: &str, default: i64) -> i64 {
        match ctx.query_value(name) {
            None => default,
            Some(raw) => self
                .int_from(vec![json!("query"), json!(name)], &json!(raw))
                .unwrap_or(default),
        }
    }

    pub fn query_float(&mut self, ctx: &Ctx, name: &str) -> Option<f64> {
        let raw = ctx.query_value(name)?;
        self.float_from(vec![json!("query"), json!(name)], &json!(raw))
    }

    pub fn query_bool(&mut self, ctx: &Ctx, name: &str) -> bool {
        match ctx.query_value(name) {
            None => false,
            Some(raw) => self
                .bool_from(vec![json!("query"), json!(name)], &json!(raw))
                .unwrap_or(false),
        }
    }
}

pub struct Body<'a> {
    pub check: &'a mut Check,
    fields: Option<Map<String, Value>>,
    whole: Value,
}

impl<'a> Body<'a> {
    pub fn open(check: &'a mut Check, body: BodyJson) -> Body<'a> {
        let (fields, whole) = match body {
            BodyJson::Absent | BodyJson::Json(Value::Null) => {
                check.error(
                    "missing",
                    vec![json!("body")],
                    "Field required",
                    &Value::Null,
                );
                (None, Value::Null)
            }
            BodyJson::Json(Value::Object(map)) => (Some(map.clone()), Value::Object(map)),
            BodyJson::Json(other) => {
                check.error(
                    "model_attributes_type",
                    vec![json!("body")],
                    "Input should be a valid dictionary or object to extract fields from",
                    &other,
                );
                (None, other)
            }
            BodyJson::Raw(text) => {
                let input = Value::String(text);
                check.error(
                    "model_attributes_type",
                    vec![json!("body")],
                    "Input should be a valid dictionary or object to extract fields from",
                    &input,
                );
                (None, input)
            }
        };
        Body {
            check,
            fields,
            whole,
        }
    }

    pub fn raw(&self) -> Option<&Map<String, Value>> {
        self.fields.as_ref()
    }

    fn loc(name: &str) -> Vec<Value> {
        vec![json!("body"), json!(name)]
    }

    fn get(&mut self, name: &str, required: bool) -> Option<Value> {
        let fields = self.fields.as_ref()?;
        match fields.get(name) {
            Some(value) => Some(value.clone()),
            None => {
                if required {
                    let whole = self.whole.clone();
                    self.check
                        .error("missing", Self::loc(name), "Field required", &whole);
                }
                None
            }
        }
    }

    pub fn string(
        &mut self,
        name: &str,
        min: usize,
        max: Option<usize>,
        default: Option<&str>,
    ) -> Option<String> {
        let Some(value) = self.get(name, default.is_none()) else {
            return default.map(str::to_string);
        };
        let Some(text) = value.as_str() else {
            self.check.error(
                "string_type",
                Self::loc(name),
                "Input should be a valid string",
                &value,
            );
            return None;
        };
        let length = text.chars().count();
        if length < min {
            let s = if min == 1 { "" } else { "s" };
            self.check.error_ctx(
                "string_too_short",
                Self::loc(name),
                format!("String should have at least {min} character{s}"),
                &value,
                json!({"min_length": min}),
            );
            return None;
        }
        if let Some(max) = max.filter(|m| length > *m) {
            let s = if max == 1 { "" } else { "s" };
            self.check.error_ctx(
                "string_too_long",
                Self::loc(name),
                format!("String should have at most {max} character{s}"),
                &value,
                json!({"max_length": max}),
            );
            return None;
        }
        Some(text.to_string())
    }

    pub fn object(&mut self, name: &str) -> Option<Value> {
        let value = self.get(name, true)?;
        if value.is_object() {
            Some(value)
        } else {
            self.check.error(
                "dict_type",
                Self::loc(name),
                "Input should be a valid dictionary",
                &value,
            );
            None
        }
    }

    pub fn boolean(&mut self, name: &str) -> Option<bool> {
        let value = self.get(name, true)?;
        self.check.bool_from(Self::loc(name), &value)
    }

    pub fn float(&mut self, name: &str, low: Option<f64>, high: Option<f64>) -> Option<f64> {
        let value = self.get(name, true)?;
        let parsed = self.check.float_from(Self::loc(name), &value)?;
        self.check
            .range(Self::loc(name), parsed, &value, low, high, false)
            .then_some(parsed)
    }

    pub fn int(
        &mut self,
        name: &str,
        low: Option<i64>,
        high: Option<i64>,
        default: Option<i64>,
    ) -> Option<i64> {
        let Some(value) = self.get(name, default.is_none()) else {
            return default;
        };
        let parsed = self.check.int_from(Self::loc(name), &value)?;
        self.check
            .range(
                Self::loc(name),
                parsed as f64,
                &value,
                low.map(|l| l as f64),
                high.map(|h| h as f64),
                true,
            )
            .then_some(parsed)
    }

    pub fn int_list(&mut self, name: &str, min: usize, max: usize) -> Option<Vec<i64>> {
        let value = self.get(name, true)?;
        let Some(items) = value.as_array() else {
            self.check.error(
                "list_type",
                Self::loc(name),
                "Input should be a valid list",
                &value,
            );
            return None;
        };
        let mut out = Vec::with_capacity(items.len());
        let mut ok = true;
        for (i, item) in items.iter().enumerate() {
            match self
                .check
                .int_from(vec![json!("body"), json!(name), json!(i)], item)
            {
                Some(n) => out.push(n),
                None => ok = false,
            }
        }
        if !ok {
            return None;
        }
        let plural = |n: usize| if n == 1 { "item" } else { "items" };
        if out.len() < min {
            self.check.error_ctx(
                "too_short",
                Self::loc(name),
                format!(
                    "List should have at least {min} {} after validation, not {}",
                    plural(min),
                    out.len()
                ),
                &value,
                json!({"field_type": "List", "min_length": min, "actual_length": out.len()}),
            );
            return None;
        }
        if out.len() > max {
            self.check.error_ctx(
                "too_long",
                Self::loc(name),
                format!(
                    "List should have at most {max} {} after validation, not {}",
                    plural(max),
                    out.len()
                ),
                &value,
                json!({"field_type": "List", "max_length": max, "actual_length": out.len()}),
            );
            return None;
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lax_parsing_follows_pydantic() {
        assert_eq!(parse_int_text(" 5 "), Some(5));
        assert_eq!(parse_int_text("5.0"), Some(5));
        assert_eq!(parse_int_text("5.5"), None);
        assert_eq!(parse_int_text("abc"), None);
        assert_eq!(parse_float_text("1e3"), Some(1000.0));
        assert_eq!(parse_float_text("inf"), None);
        assert_eq!(parse_bool_text("YES"), Some(true));
        assert_eq!(parse_bool_text("off"), Some(false));
        assert_eq!(parse_bool_text("2"), None);
    }

    #[test]
    fn json_errors_point_where_python_does() {
        let at = |text: &str| {
            let error = serde_json::from_str::<Value>(text).unwrap_err();
            match json_invalid(text.as_bytes(), &error) {
                ApiError::Validation(e) => (
                    e[0]["loc"][1].as_u64().unwrap(),
                    e[0]["ctx"]["error"].as_str().unwrap().to_string(),
                ),
                _ => unreachable!(),
            }
        };
        assert_eq!(
            at("{"),
            (
                1,
                "Expecting property name enclosed in double quotes".into()
            )
        );
        assert_eq!(at("[1,"), (3, "Expecting value".into()));
        assert_eq!(
            at("{x"),
            (
                1,
                "Expecting property name enclosed in double quotes".into()
            )
        );
        assert_eq!(at("{} x"), (3, "Extra data".into()));
    }

    #[test]
    fn body_fields_collect_every_error() {
        let mut check = Check::new();
        let mut body = Body::open(
            &mut check,
            BodyJson::Json(
                json!({"code": "ab", "minutes": 2000, "ids": [], "on": "maybe", "x": 1}),
            ),
        );
        assert_eq!(body.string("code", 4, Some(64), None), None);
        assert_eq!(body.float("minutes", Some(0.0), Some(1440.0)), None);
        assert_eq!(body.int_list("ids", 1, 200), None);
        assert_eq!(body.boolean("on"), None);
        assert_eq!(body.object("credential"), None);
        assert_eq!(body.int("seconds", Some(5), Some(300), Some(30)), Some(30));
        let errors = check.errors;
        let kinds: Vec<&str> = errors.iter().map(|e| e["type"].as_str().unwrap()).collect();
        assert_eq!(
            kinds,
            [
                "string_too_short",
                "less_than_equal",
                "too_short",
                "bool_parsing",
                "missing"
            ]
        );
        assert_eq!(
            errors[1]["msg"],
            "Input should be less than or equal to 1440"
        );
        assert_eq!(errors[1]["ctx"], json!({"le": 1440.0}));
        assert_eq!(errors[0]["ctx"], json!({"min_length": 4}));
    }

    #[test]
    fn non_objects_and_absent_bodies_are_reported_once() {
        let mut check = Check::new();
        let mut body = Body::open(&mut check, BodyJson::Json(json!([1])));
        assert_eq!(body.string("mode", 0, None, None), None);
        assert_eq!(check.errors.len(), 1);
        let mut check = Check::new();
        Body::open(&mut check, BodyJson::Absent);
        assert_eq!(check.errors[0]["type"], "missing");
    }
}
