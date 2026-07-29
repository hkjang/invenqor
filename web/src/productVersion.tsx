export type SystemInfo = {
  database_mode: string;
  server_version: string;
  commit: string;
  build_time: string;
  agent_auto_enrollment?: boolean;
  agent_enrollment_mode?: "open"|"token"|"disabled";
  agent_enrollment_source?: "database"|"startup-environment";
  agent_enrollment_policy_available?: boolean;
  listen_address?: string;
  port?: number;
};

export const formatServerVersion = (value?: string) =>
  value ? `v${value.replace(/^v/i, "")}` : "확인 중";

const buildDetails = (info: SystemInfo | null) => [
  info?.commit && info.commit !== "unknown" ? `Commit ${info.commit.slice(0, 12)}` : "",
  info?.build_time && info.build_time !== "unknown" ? `Build ${info.build_time}` : "",
].filter(Boolean).join(" · ");

export function ProductVersion({
  info,
  compact = false,
}: {
  info: SystemInfo | null;
  compact?: boolean;
}) {
  const version = formatServerVersion(info?.server_version);
  const details = buildDetails(info);
  if (compact) {
    return <span
      className="version-chip"
      title={details || "실행 중인 Server 버전"}
      aria-label={`Invenqor Server ${version}`}
    >
      Server {version}
    </span>;
  }
  return <p className="product-version" title={details || undefined}>
    <span>INVENQOR SERVER</span>
    <strong>{version}</strong>
  </p>;
}
