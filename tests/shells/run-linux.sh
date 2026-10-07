#!/bin/sh
# Escenario de integracion de una shell dentro de un contenedor Debian.
#
# Uso: run-linux.sh <shell>   (bash, zsh, fish)
#
# Requiere: imagen Docker con expect y las shells instaladas (nombre por defecto
# nvsn-shelltest), y un binario
# nvsn de Linux. Variables:
#   NVSN_BIN   binario nvsn montado en el contenedor (por defecto ./target/debug/nvsn)
#   NVSN_ROOT  store con versions/v20.11.1/bin/node falso
#   NVSN_TEST  directorio de trabajo con "with space/.nvmrc" (ruta con espacio)
#   NVSN_IMAGE imagen Docker (por defecto nvsn-shelltest)
set -eu
SH="$1"
HERE=$(cd "$(dirname "$0")" && pwd)
BIN=${NVSN_BIN:-$HERE/../../target/debug/nvsn}
ROOT=${NVSN_ROOT:?define NVSN_ROOT con versions/v20.11.1/bin/node}
TEST=${NVSN_TEST:?define NVSN_TEST con "with space/.nvmrc"}
IMAGE=${NVSN_IMAGE:-nvsn-shelltest}
WORK="$TEST/run-$SH"
OUT="$WORK/out"
mkdir -p "$WORK/home" "$OUT"
rm -f "$OUT"/*.txt

case "$SH" in
  bash)   SPAWN="bash -i";  PROBE="sh $WORK/probe.sh >" ;;
  zsh)    SPAWN="zsh -i";   PROBE="sh $WORK/probe.sh >" ;;
  fish)   SPAWN="fish";     PROBE="sh $WORK/probe.sh >" ;;
  *) echo "shell no soportada: $SH (bash, zsh, fish)" >&2; exit 2 ;;
esac

cp "$HERE/probe.sh" "$WORK/probe.sh"
sed -e "s#@SPAWN@#$SPAWN#" -e "s#@CD@#cd#" \
    -e "s#@PROBE@#$PROBE#" -e "s#@W@#$TEST/with space#g" -e "s#@OUT@#$OUT#g" \
    "$HERE/scenario.exp.in" > "$WORK/scenario.exp"

docker run --rm --network host \
  -e SHELL="/bin/$SH" -e HOME="$WORK/home" -e NVSN_DIR="$ROOT" -e TERM=dumb \
  -v "$WORK:$WORK" -v "$TEST:$TEST" -v "$ROOT:$ROOT" \
  -v "$BIN:/usr/local/bin/nvsn:ro" "$IMAGE" \
  sh -c "nvsn init $SH --apply --yes >/dev/null 2>&1; expect -f $WORK/scenario.exp"

for f in A B C D E F; do printf '%s-%s: ' "$SH" "$f"; cat "$OUT/$f.txt" 2>/dev/null || echo '(missing)'; done
