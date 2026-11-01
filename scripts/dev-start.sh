#!/usr/bin/env bash
# gw 本地开发启动器：PostgreSQL → server(8080) → admin(5173) → client(5175/Tauri)。
# 用法：./scripts/dev-start.sh [start|stop|restart|status|logs [name]]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ENV_FILE="${DEV_ENV_FILE:-$SCRIPT_DIR/dev.env}"
ENV_EXAMPLE="$SCRIPT_DIR/dev.env.example"
RUNTIME_DIR="$ROOT_DIR/.cache/dev"
PID_DIR="$RUNTIME_DIR/pids"
LOG_DIR="$RUNTIME_DIR/logs"

mkdir -p "$PID_DIR" "$LOG_DIR"

if [[ ! -f "$ENV_FILE" ]]; then
  if [[ -f "$ENV_EXAMPLE" ]]; then
    cp "$ENV_EXAMPLE" "$ENV_FILE"
    chmod 600 "$ENV_FILE"
    echo "已从 scripts/dev.env.example 生成 scripts/dev.env（本地配置，已 gitignore）"
  else
    echo "缺少 ${ENV_FILE}，请先创建" >&2
    exit 1
  fi
fi

set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

START_POSTGRES="${START_POSTGRES:-true}"
START_SERVER="${START_SERVER:-true}"
START_ADMIN="${START_ADMIN:-true}"
CLIENT_MODE="${CLIENT_MODE:-browser}"
OPEN_BROWSER="${OPEN_BROWSER:-true}"
SERVER_PORT="${SERVER_PORT:-8080}"
ADMIN_PORT="${ADMIN_PORT:-5173}"
CLIENT_PORT="${CLIENT_PORT:-5175}"
WAIT_TIMEOUT="${WAIT_TIMEOUT:-180}"
STOP_POSTGRES="${STOP_POSTGRES:-false}"
GW_LOG="${GW_LOG:-gw_server=info,tower_http=info}"
POSTGRES_SERVICE="${POSTGRES_SERVICE:-postgres}"
POSTGRES_USER="${POSTGRES_USER:-gw}"
POSTGRES_DB="${POSTGRES_DB:-gw}"
GW_DATABASE_URL="${GW_DATABASE_URL:-postgres://gw:gw@127.0.0.1:5432/gw}"
GW_MIGRATE="${GW_MIGRATE:-true}"
GW_COOKIE_SECURE="${GW_COOKIE_SECURE:-false}"

usage() {
  cat <<'USAGE'
用法: ./scripts/dev-start.sh <命令>

命令:
  start               启动 PostgreSQL 和已启用的组件（默认）
  stop                停止由本脚本启动的组件（默认保留 PostgreSQL）
  restart             先 stop 再 start
  status              查看 PID、端口、/readyz 和 Docker 状态
  logs [name]         查看日志尾部；name 可选 server/admin/client/desktop
  help                显示本帮助

配置: scripts/dev.env（首次运行自动从 dev.env.example 生成）
日志: .cache/dev/logs/
PID:  .cache/dev/pids/
USAGE
}

log()  { printf '[dev] %s\n' "$*"; }
die()  { printf '[dev] 错误: %s\n' "$*" >&2; exit 1; }

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "缺少命令: $1"
}

pid_file() { printf '%s/%s.pid' "$PID_DIR" "$1"; }
log_file() { printf '%s/%s.log' "$LOG_DIR" "$1"; }

read_pid() {
  local file
  file="$(pid_file "$1")"
  [[ -f "$file" ]] || return 1
  local pid
  pid="$(cat "$file" 2>/dev/null || true)"
  [[ -n "$pid" ]] || return 1
  printf '%s' "$pid"
}

is_pid_running() {
  local pid
  pid="$(read_pid "$1" 2>/dev/null || true)"
  [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null
}

port_in_use() {
  local port="$1"
  if command -v lsof >/dev/null 2>&1; then
    lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1
  else
    nc -z 127.0.0.1 "$port" >/dev/null 2>&1
  fi
}

check_port_available() {
  local name="$1" port="$2" pid_name="$3"
  if is_pid_running "$pid_name"; then
    log "$name 已在运行（pid $(read_pid "$pid_name")），跳过"
    return 0
  fi
  if port_in_use "$port"; then
    die "端口 $port 已被其他进程占用；先停止它或修改 scripts/dev.env（${name}）"
  fi
  return 0
}

kill_tree() {
  local pid="$1" child
  for child in $(pgrep -P "$pid" 2>/dev/null || true); do
    kill_tree "$child"
  done
  kill "$pid" 2>/dev/null || true
}

stop_component() {
  local name="$1" pidfile
  pidfile="$(pid_file "$name")"
  if [[ ! -f "$pidfile" ]]; then
    log "$name 未由本脚本启动，跳过"
    return 0
  fi
  local pid
  pid="$(cat "$pidfile" 2>/dev/null || true)"
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    log "停止 ${name}（pid ${pid}）"
    kill_tree "$pid"
    for ((_i = 0; _i < 50; _i++)); do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.1
    done
    if kill -0 "$pid" 2>/dev/null; then
      log "$name 未在 5 秒内退出，强制结束"
      kill -9 "$pid" 2>/dev/null || true
    fi
  else
    log "$name 的 PID 已失效，清理记录"
  fi
  rm -f "$pidfile"
}

start_component() {
  local name="$1" logfile pidfile
  shift
  logfile="$(log_file "$name")"
  pidfile="$(pid_file "$name")"
  : >"$logfile"
  log "启动 ${name}，日志 $logfile"
  (
    cd "$ROOT_DIR"
    nohup "$@" >>"$logfile" 2>&1 &
    echo $! >"$pidfile"
  )
  sleep 0.4
  if ! is_pid_running "$name"; then
    tail -n 30 "$logfile" >&2 || true
    die "$name 启动失败"
  fi
}

wait_http() {
  local name="$1" url="$2" timeout="${3:-$WAIT_TIMEOUT}" i
  log "等待 $name: $url"
  for ((i = 0; i < timeout * 2; i++)); do
    if curl -fsS -m 2 "$url" >/dev/null 2>&1; then
      log "$name 就绪"
      return 0
    fi
    sleep 0.5
  done
  tail -n 40 "$(log_file "$name")" >&2 || true
  die "$name 在 ${timeout}s 内未就绪；看日志 $(log_file "$name")"
}

start_postgres() {
  [[ "$START_POSTGRES" == "true" ]] || { log "跳过 PostgreSQL（START_POSTGRES=false）"; return 0; }
  command -v docker >/dev/null 2>&1 || die "START_POSTGRES=true 但找不到 docker"
  log "启动 PostgreSQL（docker compose service: ${POSTGRES_SERVICE}）"
  (cd "$ROOT_DIR" && docker compose up -d "$POSTGRES_SERVICE" >/dev/null)
  for ((_i = 0; _i < 40; _i++)); do
    if (cd "$ROOT_DIR" && docker compose exec -T "$POSTGRES_SERVICE" pg_isready -U "${POSTGRES_USER:-gw}" -d "${POSTGRES_DB:-gw}" >/dev/null 2>&1); then
      log "PostgreSQL 就绪"
      return 0
    fi
    sleep 0.5
  done
  die "PostgreSQL 未就绪；检查 docker compose ps / 端口 5432"
}

start_server() {
  [[ "$START_SERVER" == "true" ]] || { log "跳过服务端（START_SERVER=false）"; return 0; }
  local bind="${GW_BIND:-127.0.0.1:$SERVER_PORT}"
  local port="${bind##*:}"
  [[ "$port" == "8080" ]] || die "服务端端口是 ${port}，但 admin/client Vite 代理固定 8080；请改成 SERVER_PORT=8080 或同步修改 apps/*/vite.config.ts"
  export GW_BIND="$bind"
  export GW_LOG="$GW_LOG"
  local logfile
  logfile="$(log_file server)"
  check_port_available "服务端" 8080 server || return 0
  if is_pid_running server; then return 0; fi
  start_component server cargo run --manifest-path server/Cargo.toml
  wait_http server "http://127.0.0.1:8080/readyz"
}

start_admin() {
  [[ "$START_ADMIN" == "true" ]] || { log "跳过管理台（START_ADMIN=false）"; return 0; }
  if is_pid_running admin; then log "管理台已在运行"; return 0; fi
  check_port_available "管理台" "$ADMIN_PORT" admin || return 0
  start_component admin pnpm --filter @gw/admin exec vite --host 127.0.0.1 --port "$ADMIN_PORT"
  wait_http admin "http://127.0.0.1:$ADMIN_PORT/"
}

start_client() {
  case "$CLIENT_MODE" in
    none)
      log "跳过客户端（CLIENT_MODE=none）"
      ;;
    browser)
      if is_pid_running client; then log "客户端浏览器预览已在运行"; return 0; fi
      check_port_available "客户端" "$CLIENT_PORT" client || return 0
      start_component client pnpm --filter @gw/client exec vite --host 127.0.0.1 --port "$CLIENT_PORT"
      wait_http client "http://127.0.0.1:$CLIENT_PORT/"
      ;;
    desktop)
      if is_pid_running desktop; then log "桌面客户端已在运行"; return 0; fi
      check_port_available "客户端 Vite(5175)" 5175 desktop || return 0
      start_component desktop pnpm --filter @gw/client desktop:dev
      wait_http desktop "http://127.0.0.1:5175/" 240
      ;;
    *)
      die "CLIENT_MODE 只能是 browser/desktop/none，当前: $CLIENT_MODE"
      ;;
  esac
}

open_url() {
  local url="$1"
  if command -v open >/dev/null 2>&1; then
    open "$url" >/dev/null 2>&1 || true
  elif command -v xdg-open >/dev/null 2>&1; then
    xdg-open "$url" >/dev/null 2>&1 || true
  fi
}

do_start() {
  require_cmd curl
  if [[ "$START_SERVER" == "true" ]]; then require_cmd cargo; fi
  if [[ "$START_ADMIN" == "true" || "$CLIENT_MODE" != "none" ]]; then require_cmd pnpm; fi

  start_postgres
  start_server
  start_admin
  start_client

  echo
  log "启动完成"
  [[ "$START_ADMIN" == "true" ]] && echo "  管理台:  http://localhost:$ADMIN_PORT"
  if [[ "$START_SERVER" == "true" ]]; then
    echo "  服务端:  http://127.0.0.1:8080  (readyz: $(curl -fsS -m 2 http://127.0.0.1:8080/readyz 2>/dev/null || echo '未就绪'))"
  fi
  case "$CLIENT_MODE" in
    browser) echo "  客户端:  http://localhost:$CLIENT_PORT" ;;
    desktop) echo "  客户端:  桌面窗口（Vite 5175）" ;;
  esac
  echo "  首次使用：打开管理台完成 /setup，再配置供应商/模型组/发布，生成配对码"
  echo "  状态: ./scripts/dev-start.sh status    停止: ./scripts/dev-start.sh stop    日志: ./scripts/dev-start.sh logs"

  if [[ "$OPEN_BROWSER" == "true" ]]; then
    [[ "$START_ADMIN" == "true" ]] && open_url "http://localhost:$ADMIN_PORT"
    [[ "$CLIENT_MODE" == "browser" ]] && open_url "http://localhost:$CLIENT_PORT"
  fi
}

do_stop() {
  stop_component client
  stop_component desktop
  stop_component admin
  stop_component server
  if [[ "$STOP_POSTGRES" == "true" ]] && command -v docker >/dev/null 2>&1; then
    log "停止 PostgreSQL（STOP_POSTGRES=true）"
    (cd "$ROOT_DIR" && docker compose stop "$POSTGRES_SERVICE" >/dev/null || true)
  else
    log "保留 PostgreSQL（STOP_POSTGRES=false）"
  fi
}

do_status() {
  echo "配置: $ENV_FILE"
  echo "日志: $LOG_DIR"
  echo

  for name in server admin client desktop; do
    local pid
    pid="$(read_pid "$name" 2>/dev/null || true)"
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      printf '  %-8s 运行中  pid=%s\n' "$name" "$pid"
    elif [[ -n "$pid" ]]; then
      printf '  %-8s 已退出（stale pid=%s）\n' "$name" "$pid"
    else
      printf '  %-8s 未启动\n' "$name"
    fi
  done

  echo
  printf '  readyz: '
  curl -fsS -m 2 http://127.0.0.1:8080/readyz 2>/dev/null || printf '不可用'
  echo
  printf '  管理台: '
  curl -fsS -m 2 -o /dev/null -w '%{http_code}\n' "http://127.0.0.1:$ADMIN_PORT/" 2>/dev/null || echo '不可用'
  if [[ "$CLIENT_MODE" == "browser" ]]; then
    printf '  客户端: '
    curl -fsS -m 2 -o /dev/null -w '%{http_code}\n' "http://127.0.0.1:$CLIENT_PORT/" 2>/dev/null || echo '不可用'
  fi

  echo
  if command -v docker >/dev/null 2>&1; then
    (cd "$ROOT_DIR" && docker compose ps) || true
  fi
}

do_logs() {
  local name="${1:-}"
  if [[ -n "$name" ]]; then
    local file
    file="$(log_file "$name")"
    [[ -f "$file" ]] || die "没有日志: $file"
    tail -n 160 "$file"
    return 0
  fi
  for name in server admin client desktop; do
    local file
    file="$(log_file "$name")"
    if [[ -f "$file" ]]; then
      echo "===== $name ====="
      tail -n 30 "$file"
      echo
    fi
  done
}

COMMAND="${1:-start}"
case "$COMMAND" in
  start)   do_start ;;
  stop)    do_stop ;;
  restart) do_stop; do_start ;;
  status)  do_status ;;
  logs)    shift || true; do_logs "$@" ;;
  help|-h|--help) usage ;;
  *) usage; exit 2 ;;
esac
