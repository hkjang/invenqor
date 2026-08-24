#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C
export TZ=UTC

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
chart_dir="${repo_root}/deploy/helm/invenqor"
invocation_dir="$(pwd -P)"
output_arg="${1:-${repo_root}/dist/helm}"
if [[ "${output_arg}" = /* ]]; then
  output_dir="${output_arg}"
else
  output_dir="${invocation_dir}/${output_arg}"
fi

for command_name in helm tar gzip sha256sum mktemp find cp chmod awk grep mkdir mv rm realpath dirname; do
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "required command not found: ${command_name}" >&2
    exit 1
  fi
done
if ! tar --version | grep -q 'GNU tar'; then
  echo "GNU tar is required for deterministic --sort/--mtime packaging" >&2
  exit 1
fi

if [[ ! -f "${chart_dir}/Chart.yaml" ]]; then
  echo "Helm chart not found: ${chart_dir}" >&2
  exit 1
fi

chart_dir_real="$(cd "${chart_dir}" && pwd -P)"
output_dir_real="$(realpath -m "${output_dir}")"
output_probe="${output_dir_real}"
while [[ ! -d "${output_probe}" ]]; do
  if [[ -e "${output_probe}" ]]; then
    echo "output path exists but is not a directory: ${output_probe}" >&2
    exit 1
  fi
  output_parent="$(dirname "${output_probe}")"
  if [[ "${output_parent}" = "${output_probe}" ]]; then
    echo "could not resolve output directory: ${output_dir_real}" >&2
    exit 1
  fi
  output_probe="${output_parent}"
done
output_probe_real="$(cd "${output_probe}" && pwd -P)"
case "${output_probe_real}/" in
  "${chart_dir_real}/"*)
    echo "output directory must not be inside the Helm chart: ${output_dir_real}" >&2
    exit 1
    ;;
esac
output_dir="${output_dir_real}"
if find "${chart_dir}" -type l -print -quit | grep -q .; then
  echo "refusing to package a Helm chart containing symbolic links" >&2
  exit 1
fi

chart_metadata="$(helm show chart "${chart_dir}")"
chart_name="$(awk '$1 == "name:" { print $2; exit }' <<<"${chart_metadata}")"
chart_version="$(awk '$1 == "version:" { gsub(/"/, "", $2); print $2; exit }' <<<"${chart_metadata}")"
if [[ "${chart_name}" != "invenqor" ]]; then
  echo "unexpected Helm chart name: ${chart_name:-<empty>}" >&2
  exit 1
fi
if [[ ! "${chart_version}" =~ ^[0-9A-Za-z][0-9A-Za-z._+-]*$ ]]; then
  echo "unsafe or missing Helm chart version: ${chart_version:-<empty>}" >&2
  exit 1
fi

helm lint "${chart_dir}" --strict
helm template invenqor-package-validation "${chart_dir}" \
  --namespace invenqor-package-validation >/dev/null

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/invenqor-helm-package.XXXXXXXX")"
cleanup() {
  rm -rf -- "${work_dir}"
}
trap cleanup EXIT

stage_root="${work_dir}/stage"
mkdir -p "${stage_root}"
cp -R "${chart_dir}" "${stage_root}/${chart_name}"
find "${stage_root}/${chart_name}" -type d -exec chmod 0755 {} +
find "${stage_root}/${chart_name}" -type f -exec chmod 0644 {} +

archive_name="${chart_name}-${chart_version}.tgz"
archive_work="${work_dir}/${archive_name}"
checksum_work="${work_dir}/${archive_name}.sha256"

tar --sort=name \
  --format=ustar \
  --mtime='UTC 1970-01-01' \
  --owner=0 --group=0 --numeric-owner \
  -C "${stage_root}" -cf - "${chart_name}" \
  | gzip -n -9 >"${archive_work}"

# Validate the exact deterministic archive that will be published, not only its source tree.
helm show chart "${archive_work}" >/dev/null
helm lint "${archive_work}" --strict
helm template invenqor-package-validation "${archive_work}" \
  --namespace invenqor-package-validation >/dev/null

archive_hash="$(sha256sum "${archive_work}" | awk '{print $1}')"
printf '%s  %s\n' "${archive_hash}" "${archive_name}" >"${checksum_work}"

mkdir -p "${output_dir}"
mv -f -- "${archive_work}" "${output_dir}/${archive_name}"
mv -f -- "${checksum_work}" "${output_dir}/${archive_name}.sha256"

echo "Helm release package: ${output_dir}/${archive_name}"
echo "SHA-256: ${archive_hash}"
