// Fills a throwaway Server with fake inventory so the guide screenshots show
// populated screens instead of empty tables.
//
// Every value here is invented. Hostnames use the RFC 2606 `example.com`
// domain, people are `홍길동`/`hong@example.com`, and addresses stay inside
// RFC 1918 ranges, because these bytes end up printed in a PDF that is
// published with the release.

const HOSTS = [
  {
    key: "web-01",
    hostname: "web-01.demo.example.com",
    ip: "10.20.30.11",
    os: {os_family: "linux", os_name: "Ubuntu", os_version: "24.04.1 LTS", os_build: "6.8.0-45-generic", architecture: "x86_64"},
    records: [
      {asset_id: "nginx-service", category: "service", source: "systemd", payload: {name: "nginx", display_name: "A high performance web server", state: "running", active: true, enabled: true}},
      {asset_id: "nginx-process", category: "process", source: "proc", payload: {name: "nginx", executable: "/usr/sbin/nginx", pid: 1042}},
      {asset_id: "nginx-package", category: "software.package", source: "dpkg", payload: {name: "nginx", version: "1.24.0-2ubuntu7", publisher: "Ubuntu Developers"}},
      {asset_id: "docker-service", category: "service", source: "systemd", payload: {name: "docker", display_name: "Docker Application Container Engine", state: "running", active: true, enabled: true}},
      {asset_id: "docker-package", category: "software.package", source: "dpkg", payload: {name: "docker-ce", version: "27.3.1-1", publisher: "Docker Inc."}},
    ],
  },
  {
    key: "web-02",
    hostname: "web-02.demo.example.com",
    ip: "10.20.30.12",
    os: {os_family: "linux", os_name: "Ubuntu", os_version: "24.04.1 LTS", os_build: "6.8.0-45-generic", architecture: "x86_64"},
    records: [
      {asset_id: "nginx-service", category: "service", source: "systemd", payload: {name: "nginx", display_name: "A high performance web server", state: "running", active: true, enabled: true}},
      {asset_id: "nginx-package", category: "software.package", source: "dpkg", payload: {name: "nginx", version: "1.24.0-2ubuntu7", publisher: "Ubuntu Developers"}},
    ],
  },
  {
    key: "db-01",
    hostname: "db-01.demo.example.com",
    ip: "10.20.40.21",
    os: {os_family: "linux", os_name: "Rocky Linux", os_version: "9.4", os_build: "5.14.0-427.el9", architecture: "x86_64"},
    records: [
      {asset_id: "postgres-service", category: "service", source: "systemd", payload: {name: "postgresql", display_name: "PostgreSQL database server", state: "running", active: true, enabled: true}},
      {asset_id: "postgres-process", category: "process", source: "proc", payload: {name: "postgres", executable: "/usr/pgsql-16/bin/postgres", pid: 2210}},
      {asset_id: "postgres-package", category: "software.package", source: "rpm", payload: {name: "postgresql16-server", version: "16.4-1PGDG.rhel9", publisher: "PostgreSQL Global Development Group"}},
    ],
  },
  {
    key: "batch-01",
    hostname: "batch-01.demo.example.com",
    ip: "10.20.50.31",
    os: {os_family: "linux", os_name: "Red Hat Enterprise Linux", os_version: "8.10", os_build: "4.18.0-553.el8", architecture: "aarch64"},
    records: [
      {asset_id: "java-package", category: "software.package", source: "rpm", payload: {name: "java-17-openjdk", version: "17.0.12.0.7-2.el8", publisher: "Red Hat, Inc."}},
      {asset_id: "cron-service", category: "service", source: "systemd", payload: {name: "crond", display_name: "Command Scheduler", state: "running", active: true, enabled: true}},
    ],
  },
  {
    key: "win-app-01",
    hostname: "win-app-01.demo.example.com",
    ip: "10.20.60.41",
    os: {os_family: "windows", os_name: "Windows Server 2022 Standard", os_version: "21H2", os_build: "20348.2582", architecture: "x86_64"},
    records: [
      {asset_id: "mssql-service", category: "service", source: "windows_services", payload: {name: "MSSQLSERVER", display_name: "SQL Server (MSSQLSERVER)", state: "running", active: true, enabled: true, image_path: "C:\\Program Files\\Microsoft SQL Server\\MSSQL\\Binn\\sqlservr.exe -sMSSQLSERVER"}},
      {asset_id: "sqlservr-process", category: "process", source: "windows_processes", payload: {name: "sqlservr.exe", executable: "C:\\Program Files\\Microsoft SQL Server\\MSSQL\\Binn\\sqlservr.exe", pid: 4120}},
      {asset_id: "mssql-package", category: "software.package", source: "windows_registry", payload: {name: "Microsoft SQL Server 2022 (64-bit)", version: "16.0.4135.4", publisher: "Microsoft Corporation"}},
      {asset_id: "w3svc-service", category: "service", source: "windows_services", payload: {name: "W3SVC", display_name: "World Wide Web Publishing Service", state: "running", active: true, enabled: true, image_path: "C:\\Windows\\system32\\svchost.exe -k iissvcs"}},
      {asset_id: "w3wp-process", category: "process", source: "windows_processes", payload: {name: "w3wp.exe", executable: "C:\\Windows\\System32\\inetsrv\\w3wp.exe", pid: 4288}},
    ],
  },
  {
    key: "mon-01",
    hostname: "mon-01.demo.example.com",
    ip: "10.20.70.51",
    os: {os_family: "linux", os_name: "Debian GNU/Linux", os_version: "12", os_build: "6.1.0-25-amd64", architecture: "x86_64"},
    records: [
      {asset_id: "docker-service", category: "service", source: "systemd", payload: {name: "docker", display_name: "Docker Application Container Engine", state: "running", active: true, enabled: true}},
      {asset_id: "docker-package", category: "software.package", source: "dpkg", payload: {name: "docker-ce", version: "27.3.1-1", publisher: "Docker Inc."}},
    ],
  },
];

const uuid = () => crypto.randomUUID();

// A password that satisfies the Server's default policy (12+ characters with
// upper case, lower case, a digit and a symbol) and is different on every run.
// The accounts it protects live only as long as the throwaway Server, so the
// value is never recorded anywhere - not in this file, not in the output.
export const randomPassword = () => {
  const bytes = crypto.getRandomValues(new Uint8Array(24));
  const body = Buffer.from(bytes).toString("base64url").replace(/[-_]/g, "");
  return `Aa1!${body}`;
};

// A claim token that is well-formed but cannot match any enrollment secret:
// the format is the `ivq_ec_` prefix and a body of at least 32 characters.
const claimTokenFor = (body) => `ivq_ec_${body}`.padEnd(71, "0");

// `agentVersion` is printed on the Agent 목록 screen, so the caller passes the
// version of the checkout being photographed rather than a number frozen here.
export const seed = async (client, {agentVersion}) => {
  const now = Math.floor(Date.now() / 1000);
  const agents = [];
  for (const host of HOSTS) {
    const agentID = uuid();
    const enrolled = await client.json("POST", "/v1/agent/enroll", {
      agent_id: agentID,
      hostname: host.hostname,
      claim_token: claimTokenFor(host.key.replace(/[^a-z0-9]/g, "")),
    });
    const eventID = uuid();
    const records = [
      {
        asset_id: "system",
        category: "system",
        source: host.os.os_family === "windows" ? "windows_system" : "os_release",
        collected_at: now,
        payload: {hostname: host.hostname, ...host.os, primary_ip: host.ip},
      },
      ...host.records.map((record) => ({...record, collected_at: now})),
    ];
    await client.raw("POST", "/v1/agent/events", {
      schema_version: 1,
      event_id: eventID,
      agent_id: agentID,
      created_at: now,
      kind: "inventory",
      snapshot_hash: `guide-capture-${host.key}`,
      snapshot: {
        schema_version: 1,
        agent_id: agentID,
        collected_at: now,
        duration_ms: 118,
        errors: [],
        records,
      },
      changes: [],
      collection_errors: [],
    }, {
      Authorization: `Bearer ${enrolled.token}`,
      "X-Invenqor-Agent-Id": agentID,
      "X-Invenqor-Event-Id": eventID,
      "User-Agent": `invenqor-agent/${agentVersion}`,
    });
    agents.push({agentID, host});
  }

  // A rejected enrollment gives the 등록 진단 panel and the Server 로그 screen
  // something to show. Without it both screens are an empty "no records" box.
  await client.json("POST", "/v1/agent/enroll", {
    agent_id: "not-a-uuid",
    hostname: "unregistered-host.demo.example.com",
    claim_token: claimTokenFor("rejected"),
  }, {allowError: true});

  const assets = await client.json("GET", "/api/v1/assets?scope=managed&limit=200");
  const byName = new Map(assets.items.map((item) => [item.name, item]));
  const web01 = byName.get("web-01.demo.example.com");
  const db01 = byName.get("db-01.demo.example.com");
  if (web01 && db01) {
    await client.json("POST", `/api/v1/assets/${web01.id}/relations`, {
      target_asset_id: db01.id,
      relation_type: "depends_on",
    });
  }

  await client.json("POST", "/api/v1/admin/users", {
    username: "hong.gildong",
    display_name: "홍길동",
    email: "hong@example.com",
    password: randomPassword(),
    roles: ["asset_manager"],
  });
  await client.json("POST", "/api/v1/admin/users", {
    username: "kim.younghee",
    display_name: "김영희",
    email: "younghee@example.com",
    password: randomPassword(),
    roles: ["auditor"],
  });

  await client.json("POST", "/api/v1/admin/api-keys", {
    name: "cmdb-sync",
    scopes: ["assets.read", "relations.read", "queries.execute"],
  });
  await client.json("POST", "/api/v1/admin/api-keys", {
    name: "mcp-assistant",
    scopes: ["assets.read", "agents.read", "mcp.access"],
  });

  await client.json("POST", "/api/v1/query/execute", {
    query: 'type = "host" and status = "active"',
    limit: 50,
  });

  return {agents, assets: assets.items};
};

export const hostCount = HOSTS.length;
