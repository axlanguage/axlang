use crate::std_cli::StdCliPack;
use crate::std_crypto::StdCryptoPack;
use crate::std_env::StdEnvPack;
use crate::std_fs::StdFsPack;
use crate::std_io::StdIoPack;
use crate::std_json::StdJsonPack;
use crate::std_map::StdMapPack;
use crate::std_net_http::StdNetHttpPack;
use crate::std_net_http_client::StdNetHttpClientPack;
use crate::std_net_tcp::StdNetTcpPack;
use crate::std_path::StdPathPack;
use crate::std_process::StdProcessPack;
use crate::std_str::StdStrPack;
use crate::std_time::StdTimePack;
use crate::std_url::StdUrlPack;
use crate::Pack;

pub struct PackRegistry {
    packs: Vec<Box<dyn Pack>>,
}

impl Default for PackRegistry {
    fn default() -> Self {
        Self {
            packs: vec![
                Box::new(StdIoPack),
                Box::new(StdCliPack),
                Box::new(StdFsPack),
                Box::new(StdCryptoPack),
                Box::new(StdEnvPack),
                Box::new(StdProcessPack),
                Box::new(StdJsonPack),
                Box::new(StdStrPack),
                Box::new(StdPathPack),
                Box::new(StdTimePack),
                Box::new(StdUrlPack),
                Box::new(StdMapPack),
                Box::new(StdNetHttpClientPack),
                Box::new(StdNetTcpPack),
                Box::new(StdNetHttpPack),
            ],
        }
    }
}

impl PackRegistry {
    pub fn get(&self, name: &str) -> Option<&dyn Pack> {
        self.packs
            .iter()
            .find(|pack| pack.name() == name)
            .map(Box::as_ref)
    }

    pub fn by_syntax(&self, syntax: &str) -> Option<&dyn Pack> {
        self.packs
            .iter()
            .find(|pack| pack.provided_syntax().contains(&syntax))
            .map(Box::as_ref)
    }

    pub fn effects(&self) -> Vec<&'static str> {
        let mut effects = self
            .packs
            .iter()
            .flat_map(|pack| pack.provided_effects().iter().copied())
            .collect::<Vec<_>>();
        effects.sort_unstable();
        effects.dedup();
        effects
    }
}
