#!/bin/sh
# Imprime la version activa y la ruta de node visibles para el proceso hijo.
echo "V=${NVSN_VERSION:-none} N=$(command -v node || echo none)"
