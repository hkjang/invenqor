import React from "react";
import {
  CheckCircle2,
  Layers,
  Link2,
  Play,
  RefreshCw,
  ShieldQuestion,
  XCircle,
} from "lucide-react";
import {api} from "./api";
import type {AdminAccess} from "./adminPages";

type Rule = {
  id: string;
  name: string;
  description: string;
  priority: number;
  enabled: boolean;
  system_rule: boolean;
  confidence: number;
  assets: number;
  match: {
    categories?: string[];
    name_patterns?: string[];
    name_tokens?: string[];
    types?: string[];
    environments?: string[];
    attribute_equals?: Record<string, string>;
    attribute_contains?: Record<string, string>;
  };
  assign: {
    type?: string;
    environment?: string;
    criticality?: string;
    owner_department?: string;
    location?: string;
    tags?: string[];
    relate_to_host?: boolean;
    relation?: string;
  };
};

type Summary = {
  assets: number;
  classified: number;
  manual: number;
  unclassified: number;
  inferred_relations: number;
  proposed_relations: number;
};

type Proposal = {
  id: string;
  relation_type: string;
  derivation: string;
  confidence: number;
  source: {id: string; name: string; type: string; environment: string};
  target: {id: string; name: string; type: string; environment: string};
};

const derivationLabels: Record<string, string> = {
  same_agent_inventory: "같은 Agent 수집 결과",
  machine_identity: "동일 machine identifier",
};

export const CLASSIFICATION_READ_ONLY_MESSAGE =
  "settings.write 권한이 없어 분류 규칙을 변경하거나 재분류할 수 없습니다.";
export const RELATIONS_READ_ONLY_MESSAGE =
  "relations.write 권한이 없어 관계 제안을 승인하거나 거부할 수 없습니다.";
export const classificationAccessState = (access: AdminAccess) => ({
  canWriteSettings: access.superAdmin || access.permissions.includes("settings.write"),
  canWriteRelations: access.superAdmin || access.permissions.includes("relations.write"),
});

const jsonRequest = (csrf: string, body: unknown, method = "POST"): RequestInit => ({
  method,
  headers: {"Content-Type": "application/json", "X-CSRF-Token": csrf},
  body: JSON.stringify(body),
});

/**
 * The taxonomy screen answers two operator questions that nothing else could:
 * "why does this asset look like this" and "what has the product guessed that
 * I have not confirmed".
 */
export function ClassificationSettingsPanel({csrf, access}: {
  csrf: string;
  access: AdminAccess;
}) {
  const {canWriteSettings, canWriteRelations} = classificationAccessState(access);
  const [rules, setRules] = React.useState<Rule[]>([]);
  const [summary, setSummary] = React.useState<Summary|null>(null);
  const [proposals, setProposals] = React.useState<Proposal[]>([]);
  const [busy, setBusy] = React.useState(false);
  const [message, setMessage] = React.useState("");
  const [error, setError] = React.useState("");

  const load = React.useCallback(async () => {
    const [taxonomy, queue] = await Promise.all([
      api<{rules: Rule[]; summary: Summary}>("/api/v1/admin/settings/classification"),
      api<{items: Proposal[]}>("/api/v1/assets/relations/proposed")
        .catch(() => ({items: [] as Proposal[]})),
    ]);
    setRules(taxonomy.rules);
    setSummary(taxonomy.summary);
    setProposals(queue.items);
  }, []);
  React.useEffect(() => {
    load().catch(reason => setError((reason as Error).message));
  }, [load]);

  const mutate = async (work: () => Promise<unknown>, success: string) => {
    setBusy(true); setError(""); setMessage("");
    try {
      await work(); setMessage(success); await load();
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const toggle = (rule: Rule) => {
    if (!canWriteSettings) return Promise.resolve();
    return mutate(
      () => api(
      `/api/v1/admin/settings/classification/rules/${rule.id}`,
      jsonRequest(csrf, {
        enabled: !rule.enabled,
        reason: `관리 콘솔에서 ${rule.name} ${rule.enabled ? "비활성화" : "활성화"}`,
      }, "PATCH"),
      ),
      "규칙을 변경했습니다. 기존 자산에 적용하려면 재분류를 실행하십시오.",
    );
  };
  const reclassify = () => {
    if (!canWriteSettings) return Promise.resolve();
    return mutate(
      () => api(
      "/api/v1/admin/settings/classification/reclassify",
      jsonRequest(csrf, {reason: "관리 콘솔에서 재분류"}),
      ),
      "저장된 자산에 현재 규칙을 다시 적용했습니다.",
    );
  };
  const review = (proposal: Proposal, decision: "approve"|"reject") => {
    if (!canWriteRelations) return Promise.resolve();
    return mutate(
      () => api(
      `/api/v1/assets/relations/${proposal.id}/${decision}`,
      jsonRequest(csrf, {reason: `관리 콘솔 ${decision === "approve" ? "승인" : "거부"}`}),
      ),
      decision === "approve" ? "관계를 승인했습니다." : "제안을 거부했습니다.",
    );
  };

  return <article className="panel">
    <div className="panel-head">
      <h3>자산 분류·관계</h3>
      <span>수집 결과 → 업무 맥락</span>
    </div>
    <div className="settings-body classification-body">
      {error && <div className="error action-message">{error}</div>}
      {message && <div className="success action-message">
        <CheckCircle2 size={17}/>{message}
      </div>}
      {!canWriteSettings && <div id="classification-readonly-notice"
        className="admin-notice info" role="status">
        <strong>분류 설정 읽기 전용</strong><span>{CLASSIFICATION_READ_ONLY_MESSAGE}</span>
      </div>}
      {!canWriteRelations && <div id="relations-readonly-notice"
        className="admin-notice info" role="status">
        <strong>관계 제안 읽기 전용</strong><span>{RELATIONS_READ_ONLY_MESSAGE}</span>
      </div>}

      <div className="status-grid classification-summary">
        <div><span>전체 자산</span><strong>{(summary?.assets ?? 0).toLocaleString("ko-KR")}</strong></div>
        <div><span>규칙 분류</span><strong>{(summary?.classified ?? 0).toLocaleString("ko-KR")}</strong></div>
        <div><span>운영자 지정 포함</span><strong>{(summary?.manual ?? 0).toLocaleString("ko-KR")}</strong></div>
        <div><span>미분류</span><strong>{(summary?.unclassified ?? 0).toLocaleString("ko-KR")}</strong></div>
      </div>

      <div className="classification-actions">
        <button className="secondary" disabled={busy} onClick={() => load().catch(
          reason => setError((reason as Error).message),
        )}><RefreshCw size={15}/>새로고침</button>
        <button className="primary" disabled={!canWriteSettings || busy}
          aria-disabled={!canWriteSettings || busy}
          aria-describedby={!canWriteSettings ? "classification-readonly-notice" : undefined}
          title={!canWriteSettings ? CLASSIFICATION_READ_ONLY_MESSAGE : undefined}
          onClick={reclassify}>
          <Play size={15}/>저장된 자산 재분류
        </button>
      </div>
      <p className="hint">
        규칙은 우선순위 순서로 실행되며, 뒤의 규칙은 앞의 규칙이 정한 값을 조건으로
        쓸 수 있습니다. 그래서 “운영 환경의 데이터베이스는 치명”이 별도 엔진 없이
        표현됩니다. 운영자가 직접 지정한 값은 어떤 자동 실행에서도 덮어쓰지 않습니다.
      </p>

      <div className="classification-rules">
        {rules.map(rule => <div
          key={rule.id}
          className={rule.enabled ? "rule" : "rule disabled"}
        >
          <div className="rule-head">
            <b>{rule.priority}</b>
            <div>
              <strong>{rule.name}</strong>
              <small>{rule.description}</small>
            </div>
            <div className="rule-meta">
              <em>{rule.assets.toLocaleString("ko-KR")}건 적용</em>
              <span>확신도 {Math.round(rule.confidence * 100)}%</span>
            </div>
            <button disabled={!canWriteSettings || busy}
              aria-disabled={!canWriteSettings || busy}
              aria-describedby={!canWriteSettings ? "classification-readonly-notice" : undefined}
              title={!canWriteSettings ? CLASSIFICATION_READ_ONLY_MESSAGE : undefined}
              onClick={() => toggle(rule)}>
              {rule.enabled ? "비활성화" : "활성화"}
            </button>
          </div>
          <div className="rule-body">
            <div>
              <span>조건</span>
              <code>{describeMatch(rule)}</code>
            </div>
            <div>
              <span>결과</span>
              <code>{describeAssign(rule)}</code>
            </div>
          </div>
        </div>)}
        {!rules.length && <div className="admin-empty">규칙을 불러오는 중입니다.</div>}
      </div>

      <div className="section-heading classification-queue-heading">
        <div>
          <strong><ShieldQuestion size={16}/>확인이 필요한 관계 제안</strong>
          <small>
            자동으로 적용하지 않는 추론입니다. 승인하면 사람이 결정한 관계로 남고
            이후 자동 실행이 바꾸지 않습니다. 현재 확정 관계
            {" "}{(summary?.inferred_relations ?? 0).toLocaleString("ko-KR")}건.
          </small>
        </div>
      </div>
      <div className="classification-queue">
        {proposals.map(proposal => <div key={proposal.id}>
          <Link2 size={15}/>
          <div>
            <strong>
              {proposal.source.name}
              <em>{proposal.relation_type}</em>
              {proposal.target.name}
            </strong>
            <small>
              {derivationLabels[proposal.derivation] || proposal.derivation}
              {" · "}확신도 {Math.round(proposal.confidence * 100)}%
              {" · "}{proposal.source.type} / {proposal.target.type}
            </small>
          </div>
          <button disabled={!canWriteRelations || busy}
            aria-disabled={!canWriteRelations || busy}
            aria-describedby={!canWriteRelations ? "relations-readonly-notice" : undefined}
            title={!canWriteRelations ? RELATIONS_READ_ONLY_MESSAGE : undefined}
            onClick={() => review(proposal, "approve")}>
            <CheckCircle2 size={14}/>승인
          </button>
          <button className="danger" disabled={!canWriteRelations || busy}
            aria-disabled={!canWriteRelations || busy}
            aria-describedby={!canWriteRelations ? "relations-readonly-notice" : undefined}
            title={!canWriteRelations ? RELATIONS_READ_ONLY_MESSAGE : undefined}
            onClick={() => review(proposal, "reject")}>
            <XCircle size={14}/>거부
          </button>
        </div>)}
        {!proposals.length && <div className="admin-empty">
          <Layers size={16}/> 확인을 기다리는 제안이 없습니다.
        </div>}
      </div>
    </div>
  </article>;
}

/** Renders a rule's predicate as one readable line. */
export const describeMatch = (rule: Rule): string => {
  const parts: string[] = [];
  const {match} = rule;
  if (match.categories?.length) parts.push(`수집범주 ∈ ${match.categories.join(", ")}`);
  if (match.types?.length) parts.push(`유형 ∈ ${match.types.join(", ")}`);
  if (match.environments?.length) parts.push(`환경 ∈ ${match.environments.join(", ")}`);
  if (match.name_patterns?.length) parts.push(`이름 ~ ${match.name_patterns.join(" | ")}`);
  if (match.name_tokens?.length) parts.push(`이름 토큰 ∈ ${match.name_tokens.join(", ")}`);
  Object.entries(match.attribute_equals || {}).forEach(([key, value]) =>
    parts.push(`${key} = ${value}`));
  Object.entries(match.attribute_contains || {}).forEach(([key, value]) =>
    parts.push(`${key} ⊃ ${value}`));
  return parts.length ? parts.join("  ∧  ") : "모든 자산";
};

/** Renders a rule's consequence as one readable line. */
export const describeAssign = (rule: Rule): string => {
  const parts: string[] = [];
  const {assign} = rule;
  if (assign.type) parts.push(`유형 = ${assign.type}`);
  if (assign.environment) parts.push(`환경 = ${assign.environment}`);
  if (assign.criticality) parts.push(`중요도 = ${assign.criticality}`);
  if (assign.owner_department) parts.push(`담당부서 = ${assign.owner_department}`);
  if (assign.location) parts.push(`위치 = ${assign.location}`);
  if (assign.tags?.length) parts.push(`태그 + ${assign.tags.join(", ")}`);
  if (assign.relate_to_host) {
    parts.push(`호스트 관계 = ${assign.relation || "runs_on"}`);
  }
  return parts.length ? parts.join("  ·  ") : "변경 없음";
};
