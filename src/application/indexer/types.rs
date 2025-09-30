use serde::Deserialize;

#[derive(Deserialize)]
pub struct WsEvent {
    #[serde(default)]
    pub mint: Option<String>,
    #[serde(rename = "txType", default)]
    pub tx_type: Option<String>,
}
