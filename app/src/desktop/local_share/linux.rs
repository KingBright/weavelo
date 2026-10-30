//! GUI adapter only. Dropping this value leaves background sharing intact.
use crate::{
    background_service::Client,
    sharing_permissions::{SharingCall, SharingSnapshot},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
#[derive(Clone)]
pub(crate) struct Owner {
    client: Arc<Mutex<Option<Client>>>,
    profile: PathBuf,
}
impl Owner {
    pub fn new(client: Option<Client>) -> Self {
        Self {
            client: Arc::new(Mutex::new(client)),
            profile: remote_core::mesh::default_app_private_mesh_dir(),
        }
    }
    pub fn call(
        &self,
        call: SharingCall,
    ) -> impl std::future::Future<Output = Result<SharingSnapshot, String>> + Send + 'static + use<>
    {
        let owner = self.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                let current = owner.client.lock().unwrap().clone();
                let client = match current {
                    Some(client) => client,
                    None => Client::connect(&owner.profile)
                        .map_err(|_| "Sharing service is unavailable")?,
                };
                let result = client.sharing_call(call).map_err(|error| error.to_string());
                *owner.client.lock().unwrap() = if result.is_ok() { Some(client) } else { None };
                // Mutations are never replayed after an unknown acknowledgment.
                result
            })
            .await
            .unwrap_or_else(|_| Err("Sharing request stopped; check status".into()))
        }
    }
}
