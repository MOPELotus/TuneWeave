use super::*;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(try_from = "Value")]
pub(super) struct Number(pub(super) u64);
impl TryFrom<Value> for Number {
    type Error = &'static str;
    fn try_from(value: Value) -> std::result::Result<Self, Self::Error> {
        let n = match value {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.parse::<u64>().ok().filter(|n| n.to_string() == s),
            _ => None,
        };
        n.map(Self)
            .ok_or("expected a canonical nonnegative integer")
    }
}
impl Number {
    pub(super) fn id(self) -> Result<String> {
        (self.0 > 0)
            .then(|| self.0.to_string())
            .ok_or_else(malformed)
    }
}

pub(super) fn required_text(value: String) -> Result<String> {
    optional_text(Some(value), 1024)?.ok_or_else(malformed)
}
pub(super) fn optional_text(value: Option<String>, limit: usize) -> Result<Option<String>> {
    value
        .map(|v| {
            if v.len() > limit
                || v.chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            {
                return Err(malformed());
            }
            let v = v.replace("<em>", "").replace("</em>", "").trim().to_owned();
            Ok((!v.is_empty()).then_some(v))
        })
        .transpose()
        .map(Option::flatten)
}
pub(super) fn resource(id: impl Into<String>) -> Result<ResourceRef> {
    ResourceRef::new(Platform::Kugou, id).map_err(|_| malformed())
}

fn malformed() -> TuneWeaveError {
    kugou_upstream_error("KuGou catalogue returned invalid metadata")
}
