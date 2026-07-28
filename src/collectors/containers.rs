use super::{record, Collector};
use anyhow::Result;
use serde_json::json;
use std::path::Path;

pub struct ContainerCollector;

impl Collector for ContainerCollector {
    fn name(&self) -> &'static str {
        "containers"
    }

    fn is_supported(&self) -> bool {
        true
    }

    fn collect(&self, collected_at: u64) -> Result<Vec<crate::model::AssetRecord>> {
        let runtimes: Vec<_> = [
            ("docker", "/var/run/docker.sock"),
            ("containerd", "/run/containerd/containerd.sock"),
            ("podman", "/run/podman/podman.sock"),
            ("crio", "/var/run/crio/crio.sock"),
        ]
        .into_iter()
        .filter(|(_, socket)| Path::new(socket).exists())
        .map(|(name, socket)| json!({"name": name, "socket": socket}))
        .collect();

        let cgroup = std::fs::read_to_string("/proc/1/cgroup").unwrap_or_default();
        let containerized = cgroup.contains("/docker/")
            || cgroup.contains("/kubepods/")
            || cgroup.contains("/libpod-")
            || Path::new("/.dockerenv").exists()
            || Path::new("/run/.containerenv").exists();

        Ok(vec![record(
            "container.environment",
            "runtime sockets,/proc/1/cgroup",
            collected_at,
            json!({
                "host_runtime_endpoints": runtimes,
                "agent_is_containerized": containerized,
                "cgroup_version": if Path::new("/sys/fs/cgroup/cgroup.controllers").exists() { 2 } else { 1 },
                "kubernetes_service_account": Path::new("/var/run/secrets/kubernetes.io/serviceaccount").exists(),
            }),
        )])
    }
}
