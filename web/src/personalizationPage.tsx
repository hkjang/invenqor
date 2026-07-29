import React from "react";
import {Check, Gauge, Monitor, Moon, Palette, RefreshCw, Sun} from "lucide-react";
import type {
  DensityPreference,
  ThemePreference,
  UserPreferences,
} from "./preferences";

type PageOption = {id: string; label: string};

export function PersonalizationPage({
  preferences,
  pages,
  onChange,
}: {
  preferences: UserPreferences;
  pages: PageOption[];
  onChange: (preferences: UserPreferences) => void;
}) {
  const update = <K extends keyof UserPreferences>(
    key: K,
    value: UserPreferences[K],
  ) => onChange({...preferences, [key]: value});
  return <section>
    <div className="page-title"><p className="eyebrow dark">MY WORKSPACE</p>
      <h1>개인화</h1><p>내 브라우저에서만 적용되는 화면 환경과 업무 시작점을 설정합니다.</p></div>
    <div className="preference-grid">
      <PreferencePanel title="화면 테마" icon={Palette}
        description="운영 환경과 주변 밝기에 맞는 색상 모드를 선택합니다.">
        <div className="choice-grid">{([
          ["system", "시스템", Monitor],
          ["light", "라이트", Sun],
          ["dark", "다크", Moon],
        ] as [ThemePreference, string, React.ElementType][]).map(([value, label, Icon]) =>
          <button key={value} className={preferences.theme === value ? "selected" : ""}
            onClick={() => update("theme", value)}><Icon/><span>{label}</span>
            {preferences.theme === value && <Check className="choice-check"/>}</button>)}</div>
      </PreferencePanel>
      <PreferencePanel title="정보 밀도" icon={Gauge}
        description="테이블과 카드 간격을 내 화면 크기에 맞춥니다.">
        <div className="choice-grid two">{([
          ["comfortable", "여유 있게"],
          ["compact", "조밀하게"],
        ] as [DensityPreference, string][]).map(([value, label]) =>
          <button key={value} className={preferences.density === value ? "selected" : ""}
            onClick={() => update("density", value)}><span>{label}</span>
            {preferences.density === value && <Check className="choice-check"/>}</button>)}</div>
      </PreferencePanel>
      <PreferencePanel title="로그인 시작 화면" icon={Monitor}
        description="로그인 또는 새 세션에서 가장 먼저 표시할 업무 화면입니다.">
        <select value={preferences.start_page}
          onChange={event => update("start_page", event.target.value)}>
          {pages.map(page => <option key={page.id} value={page.id}>{page.label}</option>)}
        </select>
      </PreferencePanel>
      <PreferencePanel title="운영 통계 새로고침" icon={RefreshCw}
        description="대시보드가 열려 있을 때 최신 집계값을 자동으로 불러옵니다.">
        <select value={preferences.dashboard_refresh_seconds}
          onChange={event => update("dashboard_refresh_seconds", Number(event.target.value))}>
          <option value={0}>자동 새로고침 안 함</option>
          <option value={30}>30초</option><option value={60}>1분</option>
          <option value={300}>5분</option>
        </select>
      </PreferencePanel>
    </div>
    <label className="motion-preference"><input type="checkbox"
      checked={preferences.reduce_motion}
      onChange={event => update("reduce_motion", event.target.checked)}/>
      <span><strong>화면 움직임 최소화</strong>
        <small>메뉴와 상태 변화의 애니메이션을 줄여 시각적 피로를 낮춥니다.</small></span>
    </label>
    <p className="preference-note">개인화 값은 사용자 ID별로 현재 브라우저에 저장되며
      Server 설정이나 다른 사용자의 화면에는 영향을 주지 않습니다.</p>
  </section>;
}

function PreferencePanel({
  title,
  description,
  icon: Icon,
  children,
}: {
  title: string;
  description: string;
  icon: React.ElementType;
  children: React.ReactNode;
}) {
  return <article className="preference-panel"><header><Icon/><div><h3>{title}</h3>
    <p>{description}</p></div></header><div className="preference-control">{children}</div></article>;
}
