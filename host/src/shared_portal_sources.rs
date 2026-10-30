//! Local OS grants published only to the current authenticated device network.
use crate::PreparedPortalCapture;
use protocol::session::{CaptureSource, CaptureSourceInfo};
use remote_core::shared_files::ShareScope;
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct SharedPortalSources {
    scope: ShareScope,
    sources: HashMap<CaptureSource, PreparedPortalCapture>,
    display: Option<CaptureSource>,
}
impl SharedPortalSources {
    pub fn publish(
        &mut self,
        scope: ShareScope,
        captures: Vec<PreparedPortalCapture>,
        display: Option<CaptureSource>,
        current: ShareScope,
    ) -> Result<(), String> {
        if scope.is_none() || scope != current || captures.len() > 8 {
            return Err("Sharing requires the current paired device network".into());
        }
        let mut next = HashMap::new();
        for capture in captures {
            if capture.is_revoked() || next.insert(capture.source_info().source, capture).is_some()
            {
                return Err("Shared source changed or was revoked".into());
            }
        }
        if display
            .is_some_and(|id| !matches!(id, CaptureSource::Display(_)) || !next.contains_key(&id))
        {
            return Err("The selected shared display is unavailable".into());
        }
        for (id, old) in &self.sources {
            if scope != self.scope || next.get(id).is_none_or(|new| !new.same_source(old)) {
                old.revoke_source();
            }
        }
        self.scope = scope;
        self.sources = next;
        self.display = display;
        Ok(())
    }
    pub fn clear(&mut self) {
        for capture in self.sources.values() {
            capture.revoke_source();
        }
        self.sources.clear();
        self.display = None;
        self.scope = None;
    }
    pub fn reconcile(&mut self, current: ShareScope) {
        if self.scope != current {
            self.clear();
        }
    }
    pub fn capture(
        &self,
        source: CaptureSource,
        current: ShareScope,
    ) -> Option<PreparedPortalCapture> {
        if self.scope.is_none() || self.scope != current {
            return None;
        }
        let source = if source == CaptureSource::MainDisplay {
            self.display?
        } else {
            source
        };
        self.sources
            .get(&source)
            .filter(|capture| !capture.is_revoked())
            .cloned()
    }
    pub fn for_subscription(
        &self,
        source: CaptureSource,
        current: ShareScope,
        existing: Option<&PreparedPortalCapture>,
    ) -> Option<PreparedPortalCapture> {
        if self.scope.is_none() || self.scope != current {
            return None;
        }
        if source == CaptureSource::MainDisplay
            && let Some(existing) = existing
        {
            return (!existing.is_revoked()
                && self
                    .sources
                    .values()
                    .any(|parent| parent.same_source(existing)))
            .then(|| existing.clone());
        }
        self.capture(source, current)
    }
    pub fn catalog(&self, current: ShareScope) -> Vec<CaptureSourceInfo> {
        if self.scope.is_none() || self.scope != current {
            return Vec::new();
        }
        let mut sources: Vec<_> = self
            .sources
            .values()
            .filter(|capture| !capture.is_revoked())
            .map(|capture| capture.source_info())
            .collect();
        sources.sort_by_key(|info| format!("{:?}", info.source));
        sources
    }
}
impl Drop for SharedPortalSources {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn linux_existing_view_keeps_its_source_when_the_default_display_changes() {
        let (_a, a) = crate::linux_portal::runtime::tests::prepared_control_fixture().await;
        let (_b, b) = crate::linux_portal::runtime::tests::prepared_control_fixture().await;
        let scope = Some([8; 32]);
        let mut catalog = SharedPortalSources::default();
        catalog
            .publish(scope, vec![a.clone()], Some(a.source_info().source), scope)
            .unwrap();
        let viewer = catalog
            .for_subscription(CaptureSource::MainDisplay, scope, None)
            .unwrap()
            .fork()
            .unwrap();
        catalog
            .publish(
                scope,
                vec![a.clone(), b.clone()],
                Some(b.source_info().source),
                scope,
            )
            .unwrap();
        let existing = catalog
            .for_subscription(CaptureSource::MainDisplay, scope, Some(&viewer))
            .unwrap();
        assert!(existing.same_subscription(&viewer) && existing.same_source(&a));
        assert!(
            catalog
                .for_subscription(CaptureSource::MainDisplay, scope, None)
                .unwrap()
                .same_source(&b)
        );
        assert!(
            catalog
                .for_subscription(b.source_info().source, scope, Some(&viewer))
                .unwrap()
                .same_source(&b)
        );
        assert!(
            catalog
                .for_subscription(CaptureSource::MainDisplay, Some([9; 32]), Some(&viewer))
                .is_none()
        );
        a.revoke();
        assert!(
            catalog
                .for_subscription(CaptureSource::MainDisplay, scope, Some(&viewer))
                .is_none()
        );
    }
    #[tokio::test]
    async fn linux_shared_catalog_requires_current_network_and_rekey_revokes_the_real_parent() {
        let (_fixture, source) =
            crate::linux_portal::runtime::tests::prepared_control_fixture().await;
        let scope = Some([3; 32]);
        let other = Some([4; 32]);
        let id = source.source_info().source;
        let mut catalog = SharedPortalSources::default();
        assert!(
            catalog
                .publish(scope, vec![source.clone()], Some(id), other)
                .is_err()
        );
        assert!(!source.is_revoked());
        catalog
            .publish(scope, vec![source.clone()], Some(id), scope)
            .unwrap();
        assert_eq!(catalog.catalog(scope)[0].source, id);
        assert!(catalog.catalog(other).is_empty());
        assert!(catalog.capture(CaptureSource::MainDisplay, other).is_none());
        let child = catalog
            .capture(CaptureSource::MainDisplay, scope)
            .unwrap()
            .fork()
            .unwrap();
        child.revoke();
        assert!(!source.is_revoked());
        catalog.reconcile(other);
        assert!(source.is_revoked() && catalog.catalog(scope).is_empty());
    }

    #[tokio::test]
    async fn linux_shared_sources_exist_before_viewing_and_stop_independently() {
        let (_a, first) = crate::linux_portal::runtime::tests::prepared_control_fixture().await;
        let (_b, second) = crate::linux_portal::runtime::tests::prepared_control_fixture().await;
        let scope = Some([5; 32]);
        let first_id = first.source_info().source;
        let second_id = second.source_info().source;
        let mut catalog = SharedPortalSources::default();
        catalog
            .publish(
                scope,
                vec![first.clone(), second.clone()],
                Some(first_id),
                scope,
            )
            .unwrap();
        assert_eq!(catalog.catalog(scope).len(), 2);
        let viewer = catalog
            .capture(CaptureSource::MainDisplay, scope)
            .unwrap()
            .fork()
            .unwrap();
        drop(viewer);
        assert!(!first.is_revoked() && !second.is_revoked());
        catalog
            .publish(scope, vec![second.clone()], Some(second_id), scope)
            .unwrap();
        assert!(first.is_revoked() && !second.is_revoked());
        assert!(first.close_owned().await.is_ok());
        assert_eq!(catalog.catalog(scope).len(), 1);
    }
}
