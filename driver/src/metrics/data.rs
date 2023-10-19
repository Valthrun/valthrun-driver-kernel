use alloc::{
    string::String,
    vec::Vec,
};

use serde::{
    Deserialize,
    Serialize,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsReport<'a> {
    /// Unique session id for this session.
    pub session_id: &'a str,

    /// Device info
    pub device_info: &'a DeviceInfo,

    /// Entries for the report
    pub records: &'a [MetricsRecord],
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsRecord {
    // Entry sequence number
    pub seq_no: u32,

    /// timestamp is a count of 100-nanosecond intervals since January 1, 1601
    pub timestamp: u64,

    /// PCs uptime in counts of 100-nanoseconds
    pub uptime: u64,

    /// Identifyer for the type of report
    pub report_type: String,

    /// User generated payload.
    pub payload: String,
}

#[derive(Debug, Serialize)]
pub struct DeviceInfo {}

pub type RequestPostReport<'a> = MetricsReport<'a>;

#[derive(Debug, Deserialize)]
#[serde(tag = "status")]
pub enum ResponsePostReport {
    #[serde(rename = "success")]
    Success,

    #[serde(rename_all = "camelCase")]
    #[serde(rename = "rate-limited")]
    RateLimited {
        /// Retry delay in seconds
        retry_delay: u32,

        /// Sequence numbers of successfully submitted records
        records_submitted: Vec<u32>,
    },

    #[serde(rename_all = "camelCase")]
    #[serde(rename = "generic-error")]
    GenericError { drop_records: bool },
}
