# Dev Container Host Toast

Dev Container からホストに通知を送るツール  

DOIO KB16-01のバックライトで通知できるように修正

## settings.json

``` json
{
    "hooks": {
        "PermissionRequest": [
            {
                "matcher": "AskUserQuestion",
                "hooks": [
                    {
                        "type": "command",
                        "command": "~/.claude/scripts/notify.sh idle '質問に回答してください'"
                    }
                ]
            },
            {
                "matcher": "Bash|Edit|Write|NotebookEdit",
                "hooks": [
                    {
                        "type": "command",
                        "command": "~/.claude/scripts/notify.sh permission '許可が必要です'"
                    }
                ]
            }
        ],
        "Stop": [
            {
                "hooks": [
                    {
                        "type": "command",
                        "command": "~/.claude/scripts/notify.sh complete 'タスクが完了しました'"
                    }
                ]
            }
        ]
    }
}
```

## notify.sh

``` bash
#!/bin/bash

TYPE="${1:-info}"
MESSAGE="${2:-Claude Code}"

case "$TYPE" in
    "permission") TITLE="Claude Code - 許可が必要です" ;;
    "idle")       TITLE="Claude Code - 入力待ち" ;;
    "complete")   TITLE="Claude Code - 完了" ;;
    *)            TITLE="Claude Code" ;;
esac

# 通知先ホスト
#   - devcontainer 内: docker-compose の extra_hosts で windows.host を注入済み
#   - 素の WSL2: windows.host は未定義なのでデフォルトゲートウェイにフォールバック
if getent hosts windows.host >/dev/null 2>&1; then
    HOST_IP=windows.host
else
    HOST_IP=$(ip route show default 2>/dev/null | awk '/default/ {print $3; exit}')
    [ -z "$HOST_IP" ] && HOST_IP=$(grep -m1 nameserver /etc/resolv.conf | awk '{print $2}')
fi

# JSON 文字列用の最低限のエスケープ（" と \）
json_escape() {
    local s=${1//\\/\\\\}
    s=${s//\"/\\\"}
    printf '%s' "$s"
}

PAYLOAD=$(printf '{"kind":"%s","title":"%s","message":"%s"}' \
    "$(json_escape "$TYPE")" \
    "$(json_escape "$TITLE")" \
    "$(json_escape "$MESSAGE")")

curl -sS -m 5 -X POST "http://${HOST_IP}:8000/notify" \
    -H "Content-Type: application/json" \
    -d "$PAYLOAD" >/dev/null 2>&1 &

exit 0

```

## compose.yaml

``` yaml
    extra_hosts:
      - 'windows.host:${WINDOWS_HOST_IP:-host-gateway}'
```


## init.sh

``` sh
# ----------------------------------------------------------------
# Windows ホスト IP をコンテナに渡す
#
# コンテナ → WSL2 → Windows の 2 ホップ構成のため、コンテナからは
# host-gateway（= WSL2 ホスト）では Windows に届かない。
# WSL2 のデフォルトゲートウェイ = Windows ホストの IP をここで検出し、
# .devcontainer/.env に書き出す（compose が自動読み込みし extra_hosts で注入）。
# ----------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="$SCRIPT_DIR/../.env"

# set -e + pipefail 下では検出コマンドの非ゼロ終了がそのまま initializeCommand の
# 失敗になりコンテナ作成が中断する。ip コマンドが無いホスト（macOS / Docker
# Desktop）でも止めないよう `|| true` でガードし、失敗時は変数を空のままにする。
WIN_HOST_IP="$(ip route show default 2>/dev/null | awk '/default/ {print $3; exit}' || true)"

if [ -n "${WIN_HOST_IP:-}" ]; then
  touch "$ENV_FILE"

  # grep の終了コード: 0=一致あり / 1=一致なし（どちらも .tmp は正常な出力）/
  # 2=読み取り失敗等のエラー。2 のときに無条件で mv すると空の .tmp が
  # 既存 .env を上書きし、既存キーが全て消える。0/1 のときだけ差し替え、
  # それ以外は .tmp を捨てて既存 .env を残す。
  set +e
  grep -v '^WINDOWS_HOST_IP=' "$ENV_FILE" > "$ENV_FILE.tmp" 2>/dev/null
  GREP_STATUS=$?
  set -e

  if [ "$GREP_STATUS" -le 1 ]; then
    mv "$ENV_FILE.tmp" "$ENV_FILE"
  else
    rm -f "$ENV_FILE.tmp"
  fi

  echo "WINDOWS_HOST_IP=$WIN_HOST_IP" >> "$ENV_FILE"
  echo "init-host: WINDOWS_HOST_IP=$WIN_HOST_IP"
fi
```