package httpapi

// Enrollment failures are the one class of error whose audience is never
// looking at this process' stdout: the Agent is on another machine and the
// operator is in the console. Every code therefore carries both a operator
// readable summary and the concrete next action.
var enrollmentGuidance = map[string][2]string{
	"AGENT_ENROLLMENT_READY": {
		"The Agent may enroll from this source.",
		"No action is required.",
	},
	"AGENT_ENROLLMENT_SUCCEEDED": {
		"The Agent was enrolled and its host asset was created.",
		"No action is required.",
	},
	"AGENT_PREFLIGHT_READY": {
		"An Agent verified its registration path successfully.",
		"No action is required.",
	},
	"AGENT_AUTO_ENROLLMENT_DISABLED": {
		"Automatic Agent enrollment is disabled.",
		"설정 > Agent 등록 을 URL 전용 또는 토큰 모드로 바꾸거나, " +
			"Agent 화면에서 해당 장비를 직접 등록하십시오.",
	},
	"AGENT_SOURCE_NOT_ALLOWED": {
		"The Agent source IP was rejected by the enrollment policy.",
		"Add the reported source IP or its CIDR to the registration " +
			"allowlist, or register the proxy in front of it as a trusted proxy.",
	},
	"AGENT_ENROLLMENT_UNAUTHORIZED": {
		"The fleet enrollment credential was rejected.",
		"Issue a registration token in the console and write the same value " +
			"to server.enrollment_token_file on the Agent.",
	},
	"AGENT_ENROLLMENT_RATE_LIMITED": {
		"Too many Agent enrollment attempts were received.",
		"The Agent is retrying too fast; confirm one Agent process per host " +
			"and wait for the retry window to reset.",
	},
	"AGENT_ALREADY_CLAIMED": {
		"The Agent identifier is bound to another device claim.",
		"The Agent state directory was cloned into an image. Delete " +
			"agent-id and enrollment-claim.json on the clone and restart it.",
	},
	"AGENT_BLOCKED": {
		"The Agent is blocked.",
		"Unblock the Agent on the Agent page if the device is trusted again.",
	},
	"INVALID_AGENT_IDENTITY": {
		"The Agent enrollment identity is invalid.",
		"The Agent posted a malformed agent_id or claim. Upgrade the Agent " +
			"and remove a hand edited agent-id file.",
	},
	"INVALID_AGENT_SOURCE_ADDRESS": {
		"The Agent source address could not be verified.",
		"A proxy sent an unparsable X-Forwarded-For value. Correct the proxy " +
			"or remove it from the trusted proxy list.",
	},
	"INVALID_AGENT_HOSTNAME": {
		"The Agent hostname exceeded the supported length.",
		"Shorten the host name to 255 characters or fewer.",
	},
	"INVALID_AGENT_REQUEST": {
		"The Agent enrollment payload could not be decoded.",
		"A proxy or WAF is rewriting the request body. Bypass it for " +
			"/v1/agent/ paths.",
	},
	"AGENT_ENROLLMENT_POLICY_UNAVAILABLE": {
		"The automatic enrollment policy could not be loaded.",
		"메타데이터 데이터베이스에 연결할 수 없습니다. 설정 > PostgreSQL 과 " +
			"데이터베이스 health 엔드포인트를 확인하십시오.",
	},
	"AGENT_ENROLLMENT_FAILED": {
		"The server failed while creating the Agent or host asset.",
		"Inspect the recorded error detail; it is usually a database " +
			"permission or migration problem.",
	},
	"AGENT_ENDPOINT_NOT_FOUND": {
		"An Agent called a path this server does not serve.",
		"server.url on the Agent points at a wrong path or a different " +
			"product. Configure the scheme, host and port only.",
	},
	"AGENT_ENDPOINT_METHOD_NOT_ALLOWED": {
		"An Agent used the wrong HTTP method on an Agent endpoint.",
		"A proxy is rewriting the method. Allow POST to /v1/agent/ paths.",
	},
	"AGENT_UNAUTHORIZED": {
		"The Agent device credential was rejected.",
		"Delete device-credential.json in the Agent state directory so the " +
			"Agent re-enrolls, or rotate the token from the Agent page.",
	},
	"AGENT_PREFLIGHT_BLOCKED": {
		"An Agent verified its registration path and it is not usable.",
		"Open the recorded reason; it names the exact policy that rejects " +
			"this Agent.",
	},
}

func enrollmentRejectionMessage(code string) string {
	if guidance, found := enrollmentGuidance[code]; found {
		return guidance[0]
	}
	return "The Agent enrollment attempt was rejected."
}

func enrollmentRemediation(code string) string {
	if guidance, found := enrollmentGuidance[code]; found {
		return guidance[1]
	}
	return "Open the recorded diagnostic detail for this event code."
}
