#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# uid-guard — pre-commit hook that refuses to sign when user.email has no
# matching UID on user.signingkey. Vendored from the home-manager template
# so the repo carries its own copy independent of any contributor's host.

set -e

if [[ ${1:-} == --self-test ]]; then
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT
  mkdir "$work/bin"
  cp "${BASH_SOURCE[0]}" "$work/guard.sh"
  cat > "$work/bin/git" <<'MOCK'
#!/usr/bin/env bash
case "$*" in
  'config --get commit.gpgsign') echo "${UID_GUARD_TEST_SIGN-true}" ;;
  'config --get user.email') echo "${UID_GUARD_TEST_EMAIL-user@example.org}" ;;
  'config --get user.signingkey') echo "${UID_GUARD_TEST_KEY-test-key}" ;;
  *) exit 2 ;;
esac
MOCK
  cat > "$work/bin/gpg" <<'MOCK'
#!/usr/bin/env bash
printf 'uid:::::::::%s:\n' "${UID_GUARD_TEST_UID-User <user@example.org>}"
MOCK
  chmod +x "$work/bin/git" "$work/bin/gpg"
  cases=0
  check_uid() {
    local expected=$1 status=0
    shift
    env PATH="$work/bin:$PATH" "$@" bash "$work/guard.sh" > "$work/output" 2>&1 || status=$?
    if [[ $status != "$expected" ]]; then
      cat "$work/output" >&2
      echo "uid guard self-test: expected $expected, got $status" >&2
      exit 1
    fi
    cases=$((cases + 1))
  }
  check_uid 0
  check_uid 0 'UID_GUARD_TEST_UID=User <USER@EXAMPLE.ORG>'
  check_uid 1 'UID_GUARD_TEST_UID=Other <other@example.org>'
  check_uid 0 UID_GUARD_TEST_SIGN=false 'UID_GUARD_TEST_UID='
  check_uid 0 'UID_GUARD_TEST_EMAIL='
  check_uid 0 'UID_GUARD_TEST_KEY='
  echo "uid guard self-test: $cases passed"
  exit 0
fi

if ! git config --get commit.gpgsign | grep -qE '^(true|1)$'; then
  exit 0
fi

email=$(git config --get user.email || true)
signingkey=$(git config --get user.signingkey || true)

if [ -z "$email" ] || [ -z "$signingkey" ]; then
  exit 0
fi

key_ref=${signingkey%!}
matched_uid=$(gpg --list-keys --with-colons "$key_ref" 2>/dev/null \
  | awk -F: -v e="$email" 'tolower($1)=="uid" && index(tolower($10), tolower(e)) { print $10; exit }')

if [ -z "$matched_uid" ]; then
  cat >&2 <<EOF
✗ pre-commit blocked: signing-key UID mismatch.

  user.email      : $email
  user.signingkey : $signingkey
  primary key UIDs:
EOF
  gpg --list-keys --with-colons "$key_ref" 2>/dev/null \
    | awk -F: '$1=="uid" { print "    - " $10 }' >&2
  cat >&2 <<EOF

  No UID on this key contains "$email".
  Either fix user.email / user.signingkey, or attach a matching UID to the key.
EOF
  exit 1
fi

exit 0
