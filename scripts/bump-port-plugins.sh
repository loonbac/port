#!/usr/bin/env bash
# Mueve la revisión de port-plugins que fija este repo y deja el vendor de Nix en
# sintonía, en un solo paso.
#
# El acoplamiento que resuelve no es evidente: los plugins llegan como
# dependencias git (las rutas in-process de port-plugins), el `Cargo.lock` fija
# su revisión y `flake.nix` guarda ADEMÁS el hash de vendorización de esa misma
# revisión en `cargoLock.outputHashes`. Mover el pin y olvidar el hash rompe el
# `nix build` con un `hash mismatch in fixed-output derivation` que no menciona
# ni plugins ni revisión. Es exactamente el error que dejó el commit dd5bb68,
# y por eso el hash se reescribe acá con el valor que reporta el propio build.
#
# Todo el trabajo se hace en un worktree temporal: el `[patch]` local de
# desarrollo apunta a `../port-plugins`, y con el parche activo cargo resuelve a
# la ruta local y el lock nunca aprende la revisión publicada (que es la única
# que Nix puede vendorizar).
#
# Uso:
#   scripts/bump-port-plugins.sh                # tip de master, aplica y verifica
#   scripts/bump-port-plugins.sh --rev <sha>    # una revisión concreta (40 hex)
#   scripts/bump-port-plugins.sh --no-apply     # deja los commits en el worktree
#   scripts/bump-port-plugins.sh --no-verify    # no construye el paquete entero
#   scripts/bump-port-plugins.sh --push         # empuja la rama actual al final
#   scripts/bump-port-plugins.sh -h             # esta ayuda
#
# Variable de entorno:
#   PORT_INPUT   nombre del input del flake en tu configuración de NixOS. Solo se
#                usa para imprimir el último comando; por defecto `port`.
#
# El paso final necesita root y sigue siendo tuyo:
#   sudo nixos-rebuild switch --update-input "$PORT_INPUT"

set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"
MANIFEST="$ROOT/Cargo.toml"
LOCK="$ROOT/Cargo.lock"
FLAKE="$ROOT/flake.nix"
PORT_INPUT="${PORT_INPUT:-port}"

REV=""
APPLY=1
VERIFY=1
PUSH=0

usage() {
  sed -n '/^# Uso:/,/^#   sudo nixos-rebuild/p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

log() { printf '>> %s\n' "$*"; }
die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --rev) [ $# -ge 2 ] || die "--rev necesita un sha"; REV="$2"; shift 2 ;;
    --no-apply) APPLY=0; shift ;;
    --no-verify) VERIFY=0; shift ;;
    --push) PUSH=1; shift ;;
    -h|--help) usage 0 ;;
    *) die "opción desconocida: $1 (probá -h)" ;;
  esac
done

[ -f "$MANIFEST" ] || die "no encuentro $MANIFEST; ejecutá el script desde el repo"
[ -f "$FLAKE" ] || die "no encuentro $FLAKE"
# El script crea commits: sin identidad, git falla mucho más tarde y peor.
git config user.email >/dev/null 2>&1 \
  || die "git no tiene identidad configurada (user.name y user.email): el script crea commits"

# La URL sale del manifiesto: es la única verdad sobre de dónde vienen los plugins.
PLUGINS_URL="$(sed -n 's/.*git = "\(https:\/\/github\.com\/[^"]*port-plugins\.git\)".*/\1/p' "$MANIFEST" | head -1)"
[ -n "$PLUGINS_URL" ] || die "no encontré la URL de port-plugins en Cargo.toml"

# Revisión que el lock fija hoy (los dos plugins comparten repositorio y revisión).
pin_actual() {
  sed -n "s|.*${PLUGINS_URL}?branch=master#\([0-9a-f]\{40\}\)\".*|\1|p" "$LOCK" | head -1
}

if [ -n "$REV" ]; then
  target="$REV"
else
  log "resolviendo el tip de master de port-plugins"
  target="$(git ls-remote "$PLUGINS_URL" refs/heads/master | cut -f1)"
fi
[ -n "$target" ] || die "no pude resolver la revisión destino"
case "$target" in
  *[!0-9a-f]*) die "revisión inválida (se espera un sha de 40 hex): $target" ;;
esac
[ "${#target}" -eq 40 ] || die "revisión inválida (se espera un sha de 40 hex): $target"

antes="$(pin_actual)"
if [ "$antes" = "$target" ]; then
  log "el lock ya fija $target: no hay nada que hacer"
  exit 0
fi
log "moviendo el pin de ${antes:0:7} a ${target:0:7}"

BASE="$(git rev-parse HEAD)"
WORKTREE="${TMPDIR:-/tmp}/port-bump-$$"
KEEP_WORKTREE=0

cleanup() {
  if [ "$KEEP_WORKTREE" -eq 0 ]; then
    cd "$ROOT" 2>/dev/null || true
    git worktree remove --force "$WORKTREE" >/dev/null 2>&1 || true
    git worktree prune
  fi
}
trap cleanup EXIT

git worktree add --detach "$WORKTREE" HEAD >/dev/null
log "worktree temporal en $WORKTREE"

# ── 1. El pin del lock, sin el parche local ───────────────────────────────
cd "$WORKTREE"
# Desde acá el lock y el flake que importan son los del worktree, no los del
# repo principal (que sigue con el parche y con la revisión vieja).
LOCK="$WORKTREE/Cargo.lock"
FLAKE="$WORKTREE/flake.nix"
# Solo los plugins que el lock fija de verdad: `port-plugin-selection` pasó a
# ser nativo y ya no está en el grafo, así que pedirle a cargo que lo actualice
# muere con "did not match any packages" y el bump no llega a empezar.
upd=(cargo update)
for pkg in port-plugin-herdr port-plugin-selection; do
  if grep -qx "name = \"$pkg\"" "$LOCK"; then
    upd+=(-p "$pkg")
  fi
done
[ "${#upd[@]}" -gt 1 ] || die "el lock no fija ningún plugin de port-plugins"
if [ -n "$REV" ]; then
  upd+=(--precise "$target")
fi
# Sin el `if !`, un fallo de cargo mata el script en silencio por `set -e`.
if ! "${upd[@]}" 2>&1 | sed 's/^/   /'; then
  die "cargo update falló; revisá la revisión $target (¿pertenece a master de port-plugins?)"
fi

nuevo="$(sed -n "s|.*${PLUGINS_URL}?branch=master#\([0-9a-f]\{40\}\)\".*|\1|p" "$LOCK" | head -1)"
[ "$nuevo" = "$target" ] || die "cargo no movió el pin a $target (quedó en ${nuevo:-vacío}); ¿la revisión pertenece a master?"
git add Cargo.lock
git commit -q -m "chore(deps): move port-plugins to ${target:0:7}" -m "Both plugin crates share the revision of the same repository, so both move
together. The vendor hashes in flake.nix follow in the next commit: without
them the source derivation fails with a hash mismatch."

# ── 2. El hash del vendor, con el valor que reporta el build ──────────────
build_log="$(mktemp)"
log "construyendo para leer el hash que reporta Nix"
set +e
nix build .#port --no-link >"$build_log" 2>&1
status=$?
set -e

# `pipefail`: si grep no encuentra nada, la sustitución falla y `set -e` mataría
# el script antes de explicar qué pasó. Se tolera explícitamente. El `tr` saca el
# relleno con el que Nix alinea el valor del hash en su mensaje: si se cuela, el
# hash queda entre comillas con espacios y el flake deja de ser válido.
mismatch="$(grep -o 'got:[[:space:]]*sha256-[A-Za-z0-9+/=]*' "$build_log" | tail -1 || true)"
esperado="$(printf '%s' "$mismatch" | tr -d '[:space:]' | sed 's/^got://')"

if [ -n "$esperado" ]; then
  tocado=0
  for key in port-plugin-herdr port-plugin-selection; do
    # Solo las claves que el flake declara: un sed a ciegas sobre una clave que
    # no existe deja el archivo igual y el commit siguiente queda vacío.
    if grep -q "\"${key}-0\\.1\\.0\"" "$FLAKE"; then
      sed -i -E "s|(\"${key}-0\\.1\\.0\"[[:space:]]*=[[:space:]]*\")([^\"]*)(\")|\1${esperado}\3|" "$FLAKE"
      tocado=1
    fi
  done
  [ "$tocado" -eq 1 ] || die "ninguna clave de port-plugins en flake.nix coincide con el vendor"
  git add flake.nix
  git commit -q -m "fix(flake): vendor the port-plugins revision the lock pins" -m "\`cargoLock.outputHashes\` is the hash of the git source Cargo.lock pins;
moving the plugins to ${target:0:7} moved the revision by itself and left the
previous hash, which fails the source derivation with a hash mismatch."
  log "vendor actualizado a $esperado"
elif [ "$status" -eq 0 ]; then
  log "el build pasó sin tocar los hashes: ya estaban en la revisión correcta"
else
  KEEP_WORKTREE=1
  tail -5 "$build_log" | sed 's/^/   /'
  die "el build falló por otra razón; el worktree queda en $WORKTREE (log: $build_log)"
fi

if [ "$VERIFY" -eq 1 ]; then
  log "verificando el paquete completo (deja el store listo para tu rebuild)"
  if ! nix build .#port --no-link; then
    KEEP_WORKTREE=1
    die "el paquete no construye con los hashes nuevos; el worktree queda en $WORKTREE"
  fi
  log "el paquete construye"
fi

TIP="$(git rev-parse HEAD)"

# ── 3. Traer los commits a la rama actual ────────────────────────────────
if [ "$APPLY" -eq 0 ]; then
  KEEP_WORKTREE=1
  log "commits listos en el worktree (rama desacoplada, no toqué la tuya):"
  git log --oneline "$BASE..$TIP" | sed 's/^/   /'
  printf '\nPara traerlos cuando quieras:\n  git cherry-pick %s..%s\n' "$BASE" "$TIP"
  exit 0
fi

cd "$ROOT"

[ -z "$(git diff --cached --name-only)" ] || die "hay cambios preparados (staged); commitealos o deshacelos antes de correr el script"
git diff --quiet -- flake.nix || die "flake.nix tiene cambios sin commitear; guardalos antes (el script reescribe los hashes)"

if ! git diff --quiet -- Cargo.lock; then
  # El modo patch local saca las líneas `source` de los dos plugins. Cualquier
  # otro cambio en el lock es trabajo de alguien y el script no lo puede pisar.
  raras="$(git diff -- Cargo.lock \
    | grep -E '^[-+]' \
    | grep -Ev '^(---|\+\+\+)' \
    | grep -vE '^-source = "git\+.*port-plugins\.git' || true)"
  [ -z "$raras" ] || die "Cargo.lock tiene cambios que no son el modo patch; revisalos antes:
$raras"
  log "reseteo el churn del modo patch en Cargo.lock (se regenera al compilar con el [patch])"
  git checkout -- Cargo.lock
fi

log "trayendo los commits a $(git rev-parse --abbrev-ref HEAD)"
if ! git cherry-pick "$BASE..$TIP"; then
  die "el cherry-pick falló (¿conflicto?); resolvelo, o deshacelo con 'git cherry-pick --abort'"
fi

if [ "$PUSH" -eq 1 ]; then
  if command -v gh >/dev/null 2>&1 && [ -z "$(git config --get-all credential.helper 2>/dev/null || true)" ]; then
    log "empujando con el helper de gh (no hay credential.helper configurado)"
    git -c credential.helper='!gh auth git-credential' push
  else
    git push
  fi
fi

hash_final="$(sed -n 's/.*"port-plugin-herdr-0\.1\.0" *= *"\(sha256-[^"]*\)".*/\1/p' "$FLAKE" | head -1)"

# Lo que decide si el update del lock va a aceptar el cambio: nix se NIEGA a
# escribir el lock mientras este repo tenga archivos seguidos por git con cambios
# sin commitear (lo ve como "unlocked input"). Los archivos nuevos, los sin
# trackear, no cuentan.
sucios="$(git status --porcelain --untracked-files=no | awk '{print $2}' | paste -sd' ' -)"

cat <<EOF

Listo: el repo fija port-plugins en ${target:0:7} y el vendor en ${hash_final:-sin cambios}.

Falta mover tu flake.lock a esta revision, y va en dos tiempos porque el update
del lock se niega mientras este repo tenga cambios sin commitear:

  git stash push -m "wip mientras actualizo el input"   # en este repo
  nix flake update $PORT_INPUT                          # en tu config, la que consume este repo
  sudo nixos-rebuild switch
  git stash pop                                         # te devuelve tu trabajo

Los dos "git stash" sobran si el repo esta limpio. "--update-input" es el alias
viejo de ese mismo update.
EOF

if [ -n "$sucios" ]; then
  printf '\nOjo: ahora mismo hay cambios sin commitear en: %s\n' "$sucios"
  printf 'Con eso el update del lock falla; el stash de arriba no es opcional.\n'
fi
