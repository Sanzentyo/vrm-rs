# coverage-mini Playbook（gpt-5.4-mini subagent 向け）

このガイドは、カバレッジ表と進捗行の定型更新を委譲する subagent 向けに、`cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70` 実行結果から
`docs/testing.md` の `Current Coverage Snapshot` 表と `docs/progress.md` の最新カバレッジ行を反映する手順を定義します。

**通常の委譲先:** Codex subagent の `gpt-5.4-mini`。

## 使う対象ファイル

- `tools/coverage/update-coverage-docs.ps1`  
  `llvm-cov` の JSON サマリを読む/実行して、ドキュメント反映用のスニペットを生成するヘルパー。

## 1) カバレッジ取得（任意）

- 通常はヘルパーが内部で実行します（`--json --summary-only --output-path ...` を付与）。
- 既存の保存 JSON を使う場合は先に次を作成しておきます。

```powershell
cargo llvm-cov --workspace --all-features --summary-only --fail-under-lines 70 --json --output-path target/coverage-summary.json
```

## 2) プレビュー（推奨）

まずはドキュメントを書き換えず、生成内容だけ確認します。

```powershell
pwsh tools/coverage/update-coverage-docs.ps1 -SummaryJsonPath target/coverage-summary.json -Date "2026-04-30"
```

出力される内容:

- `Testing` 用の更新ブロック（`Current Coverage Snapshot` に差し込むテーブル）
- `Progress` 用の置換対象 1 行（既存の `Re-measured coverage ...` 行に対応）

## 3) 反映（必要時）

内容が合っていれば `-Apply` を付けて更新します。

```powershell
pwsh tools/coverage/update-coverage-docs.ps1 -SummaryJsonPath target/coverage-summary.json -Date "2026-04-30" -Apply
```

### 運用ルール

- `docs/testing.md` は `## Current Coverage Snapshot` セクション内のコマンド/表を更新し、表の後ろにある説明文は保持します。
- `docs/progress.md` は `- Re-measured coverage ...` で始まる最終行を置換します（最新行更新用）。
- `docs/testing.md` と `docs/progress.md` は本業務で触らない前提なので、反映後は差分を一度確認してからコミットしてください。
- 最初のスクリプト実行はドライラン（`-Apply` なし）を推奨します。ユーザーが直接反映を求めた場合、または親 Codex が生成ブロックを確認済みの場合のみ `-Apply` を使います。

## 主経路: `gpt-5.4-mini` subagent

親 Codex は、coverage 更新を narrow な subagent タスクとして委譲します。プロンプトには必ず次を含めます。

- このファイル（`docs/agents/coverage-mini.md`）に従うこと。
- `tools/coverage/update-coverage-docs.ps1` のみを使い、`docs/testing.md` と `docs/progress.md` だけを更新すること。
- 日付は CI 実行日（`YYYY-MM-DD`）に合わせること。
- 他のファイルや本体コードには触らないこと。

委譲 subagent の作業手順:

1. `cargo llvm-cov ...` を実行（または保存済み JSON を準備）。
2. `pwsh tools/coverage/update-coverage-docs.ps1 -SummaryJsonPath ...` でプレビュー。
3. 内容確認後、`pwsh tools/coverage/update-coverage-docs.ps1 -SummaryJsonPath ... -Apply` を実行。
4. `git diff docs/testing.md docs/progress.md` で更新内容を報告し、同日に対応する開発ログを記録。

## 既知の制約

- `docs/progress.md` 側は「最新行更新」なので、既存ログの行と競合しないよう 1 回の更新で 1 回ずつ使う。
- `-SummaryJsonPath` なしで実行すると時間がかかる（実行+テスト）ため、CI ローカル再現時は事前 JSON を使う運用が軽いです。
