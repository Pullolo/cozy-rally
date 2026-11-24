#!/usr/bin/env bash
# RUSTC_WRAPPER for the Android build: compiles the game's bin crate as a cdylib (the .so the
# Java activity loads), like cargo-quad-apk does. Everything else is passed through unchanged.
rustc="$1"; shift
args=("$@")
is_game=0; is_bin=0; out_dir=""; extra=""
for ((i = 0; i < ${#args[@]}; i++)); do
  case "${args[$i]}" in
    --crate-name) [ "${args[$((i+1))]}" = rally_game ] && is_game=1 ;;
    --crate-type) [ "${args[$((i+1))]}" = bin ] && is_bin=1 ;;
    --out-dir) out_dir="${args[$((i+1))]}" ;;
    -C) case "${args[$((i+1))]}" in extra-filename=*) extra="${args[$((i+1))]#extra-filename=}" ;; esac ;;
  esac
done
if [ $is_game = 1 ] && [ $is_bin = 1 ]; then
  for ((i = 0; i < ${#args[@]}; i++)); do
    [ "${args[$i]}" = --crate-type ] && args[$((i+1))]=cdylib
  done
  "$rustc" "${args[@]}" -C relocation-model=pic || exit $?
  # cargo expects the bin artifact; hand it the .so under that name
  [ -n "$out_dir" ] && [ -f "$out_dir/librally_game$extra.so" ] && cp "$out_dir/librally_game$extra.so" "$out_dir/rally_game$extra"
  exit 0
fi
exec "$rustc" "$@"
