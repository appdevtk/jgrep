# jgrep VHS demos

These demos are source-controlled terminal recordings for [VHS](https://terminaltrove.com/vhs/).

## Render all demos

Install `vhs`, `ttyd`, and `ffmpeg`, then run:

```bash
mkdir -p demos/out
vhs demos/tapes/quickstart.tape
vhs demos/tapes/yaml-and-recursive.tape
vhs demos/tapes/shortcuts-and-slurp.tape
vhs demos/tapes/counts-files-null.tape
vhs demos/tapes/pretty-and-custom-color.tape
vhs demos/tapes/explorer-snapshot.tape
vhs demos/tapes/streaming-highlight.tape
vhs demos/tapes/explore-tui-streaming.tape
```

The generated GIFs are written to `demos/out/`.

In headless containers where Chromium cannot use its sandbox, prefix the render command with `VHS_NO_SANDBOX=1`:

```bash
VHS_NO_SANDBOX=1 vhs demos/tapes/quickstart.tape
```

## Try the same commands manually

```bash
cargo build -q -p jgrep
alias jgrep=./target/debug/jgrep

jgrep '.name' demos/fixtures/users.ndjson
jgrep 'select(.age >= 18) | .name' demos/fixtures/users.ndjson
jgrep -w status=active -p name demos/fixtures/users.ndjson
jgrep -s '.role' demos/fixtures/users.ndjson
jgrep -c -w status=active demos/fixtures/users.ndjson
jgrep -l -w status=active demos/fixtures/users.ndjson demos/fixtures/logs.ndjson
jgrep -n '{count:2, files:1, null_input:true}'
jgrep --pretty '{name, role, city: .address.city}' demos/fixtures/users.ndjson
jgrep '.spec.template.spec.containers[].image' demos/fixtures/k8s/deployment.yaml
jgrep -r 'select(.kind == "Deployment" and .spec.replicas > 2) | .metadata.name' demos/fixtures/k8s
jgrep -C -w log.level=ERROR demos/fixtures/logs.ndjson
jgrep -C --color-level-field app.level '{level:.app.level,message,trace_id}' demos/fixtures/app-logs.ndjson
jgrep explore --print --filter status=active demos/fixtures/users.ndjson
jgrep completion bash | sed -n '1,12p'

JGREP_DEMO_DELAY=0.35 demos/scripts/k8s-log-stream.sh |
  jgrep -C -f demos/fixtures/k8s-log-line.jq

JGREP_DEMO_DELAY=0.45 demos/scripts/k8s-log-stream.sh |
  jgrep explore
```

In the live TUI demo, type `log.level=ERROR`, press `Ctrl-F` and `Ctrl-L`, press `Ctrl-O`, then type `{level:.log.level,message,trace_id}` to switch from filtering records to choosing a multi-field printed output.
