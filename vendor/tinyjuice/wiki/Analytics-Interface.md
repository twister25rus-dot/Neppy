# Analytics Interface

The `interface/` directory contains a self-hostable dashboard for TinyJuice
compression run metadata.

## Purpose

The UI is for inspecting:

- algorithm or compressor labels
- content kinds
- status
- original and compacted token estimates
- original and compacted bytes
- latency
- lossy flag
- CCR storage flag
- source and profile

It is not a raw prompt, context, or tool-output viewer.

## Run Locally

```sh
cd interface
npm install
npm run dev
```

Build a static bundle:

```sh
npm run build
npm run preview
```

Build and run the container:

```sh
docker build -t tinyjuice-interface .
docker run --rm -p 8080:80 tinyjuice-interface
```

## Data Contract

The UI accepts an array of records shaped like:

```json
[
  {
    "id": "run-001",
    "timestamp": "2026-07-04T15:05:00Z",
    "algorithm": "smartcrusher",
    "contentKind": "json",
    "status": "compressed",
    "originalTokens": 2840,
    "compressedTokens": 1180,
    "originalBytes": 11842,
    "compressedBytes": 4928,
    "latencyMs": 23,
    "lossy": true,
    "ccrStored": true,
    "source": "openhuman",
    "profile": "full"
  }
]
```

Sample data is demo data, not a benchmark result. Displayed reductions are
computed only from the active dataset.

## Agent Notes

- Do not upload raw content into dashboard records.
- Do not treat sample data as evidence of production savings.
- Keep status and compressor labels stable enough for dashboards, but do not
  make user-facing quality claims from them alone.
