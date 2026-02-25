#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ENV_FILE="${ROOT_DIR}/.env"

if [[ -f "${ENV_FILE}" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "${ENV_FILE}"
  set +a
fi

: "${OPENROUTER_API_KEY:?OPENROUTER_API_KEY is not set. Add it to .env or your shell env.}"

PROMPT="${1:-Hello!}"
MODEL="${MODEL:-openai/gpt-4o-mini}"

echo "Calling OpenRouter with model: ${MODEL}" >&2

# ---- Build request payload ----
request_payload="$(
  cat <<EOF
{
  "model": "${MODEL}",
  "messages": [
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "user", "content": "${PROMPT}"}
  ]
}
EOF
)"

# ---- Print full HTTP request (redacted) ----
echo "=== Full HTTP Request (Redacted) ==="
printf "POST https://openrouter.ai/api/v1/chat/completions\n"
printf "Content-Type: application/json\n"
printf "Authorization: Bearer ***REDACTED***\n\n"

if command -v jq >/dev/null 2>&1; then
  echo "${request_payload}" | jq .
else
  echo "${request_payload}"
fi

# ---- Execute request ----
response="$(
  curl -sS https://openrouter.ai/api/v1/chat/completions \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer ${OPENROUTER_API_KEY}" \
    -d "${request_payload}"
)"

# ---- Print response ----
echo "=== Response Payload ==="
if command -v jq >/dev/null 2>&1; then
  echo "${response}" | jq .
else
  echo "${response}"
fi
