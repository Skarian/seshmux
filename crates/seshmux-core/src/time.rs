use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub fn now_utc_rfc3339() -> Result<String, time::error::Format> {
    OffsetDateTime::now_utc().format(&Rfc3339)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_utc_rfc3339_formats_timestamp() {
        let value = now_utc_rfc3339().expect("timestamp");
        assert!(value.ends_with('Z'));
        assert!(value.contains('T'));
    }
}
