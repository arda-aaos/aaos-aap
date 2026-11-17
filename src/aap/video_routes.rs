//! V26 secondary video route validation and lifecycle scaffold.
//!
//! This module NEVER opens a media channel or starts a video sender.
//! MAIN behavior remains the production legacy path. Subsequent versions can
//! connect each validated route to independent capture and encoder resources.
use std::collections::HashSet;

use super::proto::MediaSinkSelection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoRole {
    Main,
    Cluster,
    Auxiliary,
    Unknown,
}

impl VideoRole {
    fn from_wire(text: &str) -> Self {
        match text {
            "DISPLAY_TYPE_MAIN" => Self::Main,
            "DISPLAY_TYPE_CLUSTER" => Self::Cluster,
            "DISPLAY_TYPE_AUXILIARY" => Self::Auxiliary,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ChannelPhase {
    Discovered,
    OpenRequested,
    Opened,
    Configured,
    Streaming,
    Suspended,
    Closed,
    Failed,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Route {
    pub service_id: u8,
    pub display_id: u32,
    pub role: VideoRole,
    pub phase: ChannelPhase,
    pub selected_config_index: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct RoutePlan {
    pub main_id: u8,
    pub routes: Vec<Route>,
    pub warnings: Vec<String>,
    pub secondary_enabled: bool,
}

/// Fail closed if the discovered MAIN is not the existing MAIN video session.
/// Preserve the first-video selection and existing sender unmodified.
/// Secondary routes are only inert descriptors until capture/ACK/focus wiring exists.
pub fn plan(
    sinks: &[MediaSinkSelection],
    legacy_main_id: Option<i32>,
) -> Result<RoutePlan, String> {
    let Some(legacy_main_id) = legacy_main_id else {
        return Err("no_legacy_video_sink".into());
    };
    let mut ids = HashSet::new();
    let mut main = 0;
    let mut cluster = 0;
    let mut routes = Vec::new();
    let mut warnings = Vec::new();
    for sink in sinks {
        if sink.service_id < 1 || sink.service_id > u8::MAX as i32 {
            return Err(format!("invalid_service_id:{}", sink.service_id));
        }
        if !ids.insert(sink.service_id) {
            return Err(format!("duplicate_service_id:{}", sink.service_id));
        }
        let role = VideoRole::from_wire(&sink.display_type);
        match role {
            VideoRole::Main => main += 1,
            VideoRole::Cluster => cluster += 1,
            VideoRole::Unknown => warnings.push(format!(
                "unknown_display_role:service={}", sink.service_id
            )),
            VideoRole::Auxiliary => (),
        }
        routes.push(Route {
            service_id: sink.service_id as u8,
            display_id: sink.display_id,
            role,
            phase: ChannelPhase::Discovered,
            selected_config_index: sink.selected_video_config.as_ref().map(|c| c.config_index),
        });
    }
    if main != 1 {
        return Err(format!("expected_one_main:found={main}"));
    }
    if cluster > 1 {
        return Err(format!("expected_at_most_one_cluster:found={cluster}"));
    }
    let Some(main_route) = routes.iter().find(|r| r.role == VideoRole::Main) else {
        return Err("main_missing".into());
    };
    if legacy_main_id != main_route.service_id as i32 {
        return Err(format!(
            "legacy_selected_not_main:legacy={} expected={}",
            legacy_main_id, main_route.service_id
        ));
    }
    let main_id = main_route.service_id;
    // No extra service is opened: this is a dry-run route/phase model.
    Ok(RoutePlan { main_id, routes, warnings, secondary_enabled: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink(id: i32, role: &str, display: u32) -> MediaSinkSelection {
        MediaSinkSelection {
            service_id: id,
            available_type: "MEDIA_CODEC_VIDEO_H264".into(),
            display_id: display,
            display_type: role.into(),
            video_configs: 0,
            first_video_config: None,
            video_config_selections: vec![],
            selected_video_config: None,
        }
    }
    #[test]
    fn main_and_cluster_and_multiple_aux() {
        let sinks = [
            sink(1, "DISPLAY_TYPE_MAIN", 0),
            sink(7, "DISPLAY_TYPE_CLUSTER", 1),
            sink(9, "DISPLAY_TYPE_AUXILIARY", 2),
            sink(11, "DISPLAY_TYPE_AUXILIARY", 3),
        ];
        let p = plan(&sinks, Some(1)).unwrap();
        assert_eq!(p.routes.len(), 4);
        assert_eq!(p.main_id, 1);
        assert!(!p.secondary_enabled);
        assert_eq!(p.routes[1].role, VideoRole::Cluster);
        assert_eq!(p.routes[3].role, VideoRole::Auxiliary);
    }
    #[test]
    fn no_cluster_is_ok() {
        assert!(plan(&[sink(1, "DISPLAY_TYPE_MAIN", 0)], Some(1)).is_ok());
    }
    #[test]
    fn duplicate_cluster_fails() {
        let sinks = [sink(1, "DISPLAY_TYPE_MAIN", 0),sink(2,"DISPLAY_TYPE_CLUSTER",1),sink(3,"DISPLAY_TYPE_CLUSTER",2)];
        assert!(plan(&sinks,Some(1)).unwrap_err().contains("at_most_one_cluster"));
    }
    #[test]
    fn duplicate_service_id_fails() {
        let sinks = [sink(1, "DISPLAY_TYPE_MAIN", 0),sink(1,"DISPLAY_TYPE_AUXILIARY",2)];
        assert!(plan(&sinks,Some(1)).unwrap_err().contains("duplicate_service_id"));
    }
    #[test]
    fn legacy_non_main_fails_closed() {
        let sinks = [sink(9,"DISPLAY_TYPE_CLUSTER",1),sink(1, "DISPLAY_TYPE_MAIN",0)];
        assert!(plan(&sinks,Some(9)).unwrap_err().contains("legacy_selected_not_main"));
    }
    #[test]
    fn invalid_service_id_fails() {
        assert!(plan(&[sink(0,"DISPLAY_TYPE_MAIN",0)],Some(0)).is_err());
    }
}
