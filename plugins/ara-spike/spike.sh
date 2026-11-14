#!/bin/zsh
# The whole spike check. usage: spike.sh   (exit 0 = every line passed)
# DS_ARA_DEPS names the folder with the built parts (default: .deps next to this script):
#   build-examples/bin/Debug/ARATestHost, DrumSpikeAraSpike.clap, build-wrapper/DrumSpikeAraSpike.vst3
cd "$(dirname "$0")" || exit 2
D=${DS_ARA_DEPS:-$PWD/.deps}; O=$PWD/out; mkdir -p $O
export CLAP_PATH=$D              # the VST3 wrapper finds the CLAP here; nothing is installed into the user's plugin folders
H=$D/build-examples/bin/Debug/ARATestHost; F=$PWD/fixtures; fail=0
[ -x $H ] || { echo "FAIL: no test host at $H (see README, build notes)"; exit 2; }
/usr/bin/python3 -I check.py make || exit 2
host() {  # host <label> <args...> : run the test host with a 60 s guard
  label=$1; shift; "$H" "$@" > $O/run-$label.log 2>&1 & pid=$!; n=0
  while kill -0 $pid 2>/dev/null; do /bin/sleep 1; n=$((n+1)); [ $n -ge 60 ] && { kill $pid; echo "FAIL $label: host timeout"; return 9; }; done
  wait $pid; rc=$?; [ $rc -eq 0 ] || echo "FAIL $label: host rc=$rc"; return $rc
}
for kind in clap vst3; do
  if [ $kind = clap ]; then B=$D/DrumSpikeAraSpike.clap; else B=$D/build-wrapper/DrumSpikeAraSpike.vst3; fi
  echo "=== $kind: ${B#$D/}"
  # 1. the plugin reads every sample of four sources and gives the hits back as notes
  out=$O/out-$kind.jsonl; rm -f $out
  DS_ARA_SPIKE_OUT=$out host read-$kind -$kind $B -file $F/kick_48k_mono.wav $F/snare_44k_stereo.wav $F/tom_96k_mono.wav $F/hat_48k_mono.wav -test ContentReading \
    && /usr/bin/python3 -I check.py verify $out $kind || fail=1
  # 2. the host renders two regions back to back through the plugin instance; the audio must be the spikes
  dump=$O/render-$kind.f32; rm -f $dump $dump.txt
  DS_ARA_SPIKE_OUT=$O/out-render-$kind.jsonl DS_ARA_SPIKE_RENDER=$dump host render-$kind -$kind $B -file $F/kick_48k_mono.wav $F/hat_48k_mono.wav -test PlaybackRendering \
    && /usr/bin/python3 -I check.py render $dump render-$kind kick_48k_mono.wav hat_48k_mono.wav || fail=1
  # 3. the other host tests that touch the plugin instance and the model
  for t in EditorView PropertyUpdates Archiving; do
    host $t-$kind -$kind $B -file $F/kick_48k_mono.wav -test $t && echo "ok   $kind $t: host exit 0" || fail=1
  done
done
[ $fail -eq 0 ] && echo "SPIKE: PASS (CLAP and wrapped VST3)" || echo "SPIKE: FAIL"
exit $fail
