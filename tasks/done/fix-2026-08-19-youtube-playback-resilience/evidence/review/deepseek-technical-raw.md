# voice:deepseek-technical provider:deepseek-coder ts:2026-08-19T18:26:59.880Z
- Priority: P0
- Severity: major
- File/area: src-tauri/src/ytdlp.rs
- Issue: The `resolve_stream_info` function does not handle the `ResolverStrategy` enum correctly, leading to potential runtime errors.
- Fix assessment: The `resolve_stream_info` function should be updated to properly handle the `ResolverStrategy` enum, ensuring that each strategy is applied correctly and that errors are reported appropriately.
