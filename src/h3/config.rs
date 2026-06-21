use crate::config::ServerConfig;

#[derive(Clone)]
pub struct H3Config {
    pub udp_listen: String,
}

impl H3Config {
    pub fn new(udp_listen: String) -> Self {
        Self { udp_listen }
    }

    pub fn from_server(server: &ServerConfig) -> anyhow::Result<Self> {
        Ok(Self {
            udp_listen: server.h3_udp_listen.clone(),
        })
    }
}
