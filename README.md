# Scanner Bridge

Local companion app that bridges the Decoda Console to physical document scanners.

## Development

```bash
# Run with mock scanner (no hardware needed)
npm run dev

# Run with real scanner
cargo tauri dev

# Build for distribution
npm run build
```

## Architecture

- **Tauri v2** system tray app (Rust backend, minimal HTML frontend)
- **axum** HTTP + WebSocket server on `localhost:11235`
- Platform scanner backends: ImageCaptureCore (macOS), WIA 2.0 (Windows)
- Mock backend for development (`--mock` flag)

## API

- `GET /health` — Check if bridge is running
- `GET /scanners` — List connected scanners
- `WS /ws` — WebSocket for scan commands and image delivery
