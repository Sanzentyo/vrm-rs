# 現状レンダリング画像比較ボード

更新日: 2026-06-21

この Markdown は、いまワークスペースに存在するレンダリング結果画像を横並びで確認するためのレビュー用ページです。画像と raw 比較レポートは `.external-fixtures/` または `target/` 配下のローカル artifact で、リポジトリにはコミットしません。

基準画像は `three-vrm`、比較対象は Rust 側の `wgpu` / Bevy / Ash です。PNG は目視レビュー用で、数値判断は direct `.imqraw` 比較と audit JSON/Markdown を優先します。

## 現時点の読み

| 観点 | 現状 |
| --- | --- |
| Alpha / 透明 | 主要セットでは wgpu / Bevy / Ash とも alpha mismatch は 0。透明 blend guard も維持。 |
| wgpu と Ash | Seed-san の現行基準セットでは同値に近い。expanded diagnostic でも actual RGB はかなり近い。 |
| Bevy | alpha は一致。直近の expanded residual join では wgpu/Ash との差もかなり縮まり、以前の selected-sample 寄りの読みは現行 artifact では弱まっている。 |
| glTF/PBR | generated guard は良好。Seed-san では `backpack_nm` / `node145/mesh4/prim9/base` に top residual が集中。 |
| MToon | `eye`、`arm_mat`、`arm_plastic`、`huku_bake` 周辺に局所差分が残る。 |
| 色差の形 | `gltf_pbr` / `mtoon` とも、現行 residual join では global gain より additive RGB offset の方が近い。次は ambient/fill/light accumulation を疑う。 |

## まず見る画像セット

| 用途 | ディレクトリ | 見るポイント |
| --- | --- | --- |
| 現在の Seed-san 基準セット | [`../.external-fixtures/render-parity-ash-current-base-uv-rerun`](../.external-fixtures/render-parity-ash-current-base-uv-rerun) | three-vrm / wgpu / Bevy / Ash の全体像 |
| 実サンプル sweep | [`../.external-fixtures/render-parity-samples-ash-gated-check`](../.external-fixtures/render-parity-samples-ash-gated-check) | Seed-san、constraint、UV animation、expression、VRM0 |
| 透明 blend guard | [`../.external-fixtures/render-parity-transparent-generated-ash-gated`](../.external-fixtures/render-parity-transparent-generated-ash-gated) | alpha / blend regression |
| glTF/PBR fallback guard | [`../.external-fixtures/render-parity-gltf-pbr-generated`](../.external-fixtures/render-parity-gltf-pbr-generated) | MToon 以外の material path |
| Seed-san blocker diagnostic | [`../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback) / [`../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback) | material/color/ownership の残差 |

## 現在の Seed-san 基準セット

Artifact:
[`../.external-fixtures/render-parity-ash-current-base-uv-rerun`](../.external-fixtures/render-parity-ash-current-base-uv-rerun)

Metric: `rgb-visible`。背景は opaque black。alpha mismatch は wgpu / Bevy / Ash ともに `0` です。

| Renderer | PSNR | Gradient-domain PSNR | Changed RGB pixels | Max channel delta | Alpha mismatches |
| --- | ---: | ---: | ---: | ---: | ---: |
| `wgpu` | 36.8913 | 32.6698 | 1107 | 251 | 0 |
| Bevy | 36.8708 | 32.6368 | 9252 | 251 | 0 |
| Ash | 36.8913 | 32.6698 | 1107 | 251 | 0 |

| three-vrm reference | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/three-vrm/Seed-san.frame000.png" width="190"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/wgpu/Seed-san.frame000.png" width="190"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/bevy/Seed-san.frame000.png" width="190"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/ash/Seed-san.frame000.png" width="190"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/diff/Seed-san.wgpu-vs-three-vrm.diff.png" width="190"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/diff/Seed-san.bevy-vs-three-vrm.diff.png" width="190"> | <img src="../.external-fixtures/render-parity-ash-current-base-uv-rerun/diff/Seed-san.ash-vs-three-vrm.diff.png" width="190"> |

Raw reports:

- [`Seed-san.wgpu-vs-three-vrm.imqraw-rust.json`](../.external-fixtures/render-parity-ash-current-base-uv-rerun/reports/Seed-san.wgpu-vs-three-vrm.imqraw-rust.json)
- [`Seed-san.bevy-vs-three-vrm.imqraw-rust.json`](../.external-fixtures/render-parity-ash-current-base-uv-rerun/reports/Seed-san.bevy-vs-three-vrm.imqraw-rust.json)
- [`Seed-san.ash-vs-three-vrm.imqraw-rust.json`](../.external-fixtures/render-parity-ash-current-base-uv-rerun/reports/Seed-san.ash-vs-three-vrm.imqraw-rust.json)

## 実サンプル sweep

Artifact:
[`../.external-fixtures/render-parity-samples-ash-gated-check`](../.external-fixtures/render-parity-samples-ash-gated-check)

Metric: `rgb-visible`。

| Fixture | wgpu PSNR | Bevy PSNR | Ash PSNR | 状態 |
| --- | ---: | ---: | ---: | --- |
| `Seed-san.vrm` | 34.6538 | 34.1163 | 34.6391 | gate は通過。主な実モデル color / ownership target。 |
| `VRM1_Constraint_Twist_Sample.vrm` | 36.2518 | 36.2349 | 36.2509 | constraint path の広域 regression は見えない。 |
| `VRMC_materials_mtoon_UV_Animation_Test.vrm` | 35.6342 | 35.6202 | 35.6342 | UV animation path をカバー。 |
| `VRMC_vrm_expressions_isBinary_Overridden.vrm` | 55.6968 | 55.2106 | 55.6968 | 高い一致。 |
| `VRMC_vrm_expressions_isBinary_Overrides.vrm` | 55.7181 | 55.2306 | 55.7181 | 高い一致。 |
| `AliciaSolid_vrm-0.51.vrm` | 35.6238 | 35.6088 | 35.6238 | VRM0 compatibility path をカバー。 |

### Seed-san

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/Seed-san.frame000.png" width="175"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/Seed-san.frame000.png" width="175"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/Seed-san.frame000.png" width="175"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/Seed-san.frame000.png" width="175"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/Seed-san.wgpu-vs-three-vrm.diff.png" width="175"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/Seed-san.bevy-vs-three-vrm.diff.png" width="175"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/diff/Seed-san.ash-vs-three-vrm.diff.png" width="175"> |

### Constraint / UV Animation

| Fixture | three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- | --- |
| Constraint | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/VRM1_Constraint_Twist_Sample.frame000.png" width="145"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/VRM1_Constraint_Twist_Sample.frame000.png" width="145"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/VRM1_Constraint_Twist_Sample.frame000.png" width="145"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/VRM1_Constraint_Twist_Sample.frame000.png" width="145"> |
| MToon UV animation | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="145"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="145"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="145"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/VRMC_materials_mtoon_UV_Animation_Test.frame000.png" width="145"> |

### Expressions / VRM0

| Fixture | three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- | --- |
| Expression overridden | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/VRMC_vrm_expressions_isBinary_Overridden.frame000.png" width="130"> |
| Expression overrides | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/VRMC_vrm_expressions_isBinary_Overrides.frame000.png" width="130"> |
| AliciaSolid VRM0 | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/three-vrm/AliciaSolid_vrm-0_51.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/wgpu/AliciaSolid_vrm-0_51.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/bevy/AliciaSolid_vrm-0_51.frame000.png" width="130"> | <img src="../.external-fixtures/render-parity-samples-ash-gated-check/ash/AliciaSolid_vrm-0_51.frame000.png" width="130"> |

## 透明 / glTF-PBR guard

| Fixture | Metric | wgpu | Bevy | Ash | 状態 |
| --- | --- | ---: | ---: | ---: | --- |
| `transparent-blend_vrm` | `rgb-visible` | 54.3997 | 56.8605 | 54.3997 | alpha parity は一致。max channel delta は 1。 |
| `gltf-pbr_vrm` | `rgb-interior1px` | 47.8016 | 47.2691 | 47.8016 | non-MToon glTF/PBR fallback を guard。 |

### Generated Transparent Blend

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/three-vrm/transparent-blend_vrm.frame000.png" width="175"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/wgpu/transparent-blend_vrm.frame000.png" width="175"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/bevy/transparent-blend_vrm.frame000.png" width="175"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/ash/transparent-blend_vrm.frame000.png" width="175"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/diff/transparent-blend_vrm.wgpu-vs-three-vrm.diff.png" width="175"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/diff/transparent-blend_vrm.bevy-vs-three-vrm.diff.png" width="175"> | <img src="../.external-fixtures/render-parity-transparent-generated-ash-gated/diff/transparent-blend_vrm.ash-vs-three-vrm.diff.png" width="175"> |

### Generated glTF/PBR Fallback

| three-vrm | wgpu | Bevy | Ash |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-gltf-pbr-generated/three-vrm/gltf-pbr_vrm.frame000.png" width="200"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/wgpu/gltf-pbr_vrm.frame000.png" width="200"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/bevy/gltf-pbr_vrm.frame000.png" width="200"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/ash/gltf-pbr_vrm.frame000.png" width="200"> |

| wgpu diff | Bevy diff | Ash diff |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.wgpu-vs-three-vrm.diff.png" width="200"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.bevy-vs-three-vrm.diff.png" width="200"> | <img src="../.external-fixtures/render-parity-gltf-pbr-generated/diff/gltf-pbr_vrm.ash-vs-three-vrm.diff.png" width="200"> |

## Seed-san blocker diagnostic

このセクションは default behavior の目標ではなく、残差の原因を切り分ける診断です。expanded post-resolve は「正しい source ownership が与えられた場合に近づく pixel」を見るためのもので、blind に適用する修正ではありません。

| Set | wgpu gradient PSNR | Bevy gradient PSNR | Ash gradient PSNR | 用途 |
| --- | ---: | ---: | ---: | --- |
| Current readback | 30.6336 | 28.2142 | 35.8902 | 現在の source of truth。 |
| Expanded post-resolve diagnostic | 32.7302 | 32.4826 | 35.8901 | target-pixel coverage の診断。 |
| Second-frontier negative control | 30.9642 | 28.6200 | 35.8901 | regression guard。修正方針にはしない。 |

| three-vrm reference | wgpu current readback | Bevy current readback | Ash current readback |
| --- | --- | --- | --- |
| <img src="../.external-fixtures/render-parity-seed-base-color-flat32-outline-off-diagnostic/three-vrm/Seed-san.frame000.png" width="185"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/wgpu/Seed-san.frame000.png" width="185"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/bevy/Seed-san.frame000.png" width="185"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/ash/Seed-san.frame000.png" width="185"> |

| wgpu expanded diagnostic | Bevy expanded diagnostic | Ash expanded diagnostic |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/wgpu/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/bevy/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/ash/Seed-san.frame000.png" width="180"> |

| wgpu second-frontier negative control | Bevy second-frontier negative control | Ash second-frontier negative control |
| --- | --- | --- |
| <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback/wgpu/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback/bevy/Seed-san.frame000.png" width="180"> | <img src="../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded2-readback/ash/Seed-san.frame000.png" width="180"> |

### Shading model 別の現行 residual

Source:
[`../target/texture-draw-audit/Seed-san.shading-model-residual-join.md`](../target/texture-draw-audit/Seed-san.shading-model-residual-join.md)

| Shading model | 共有 top-residual pixels | Backend | Mean E-A | Additive RGB | Additive error | Gain RGB | Gain error | 主な surface / draw key |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | --- |
| `gltf_pbr` | 16 | Ash | 32.6468 | 17.06,19.12,20.19 | 5.4811 | 1.18,1.20,1.20 | 6.6892 | `backpack_nm`, `node145/mesh4/prim9/base` |
| `gltf_pbr` | 16 | Bevy | 33.2202 | 16.12,17.69,18.50 | 8.3446 | 1.17,1.18,1.18 | 10.4599 | `backpack_nm`, `node145/mesh4/prim9/base` |
| `gltf_pbr` | 16 | wgpu | 33.4506 | 16.31,17.88,18.88 | 8.3210 | 1.17,1.18,1.19 | 10.2461 | `backpack_nm`, `node145/mesh4/prim9/base` |
| `mtoon` | 16 | Ash | 45.6164 | 21.88,24.19,26.62 | 22.0299 | 1.19,1.21,1.24 | 23.7124 | `eye`, `arm_mat`, `arm_plastic`, `huku_bake` |
| `mtoon` | 16 | Bevy | 45.8937 | 18.19,21.44,23.88 | 27.7638 | 1.12,1.14,1.16 | 36.4266 | `eye`, `arm_mat`, `arm_plastic`, `body_nm` |
| `mtoon` | 16 | wgpu | 45.8684 | 20.81,22.62,24.81 | 24.7671 | 1.16,1.17,1.19 | 30.6092 | `eye`, `arm_mat`, `arm_plastic`, `body_nm` |

Backend agreement:

| Shading model | Pair | Shared pixels | Mean actual RGB distance | Mean E-A gap delta |
| --- | --- | ---: | ---: | ---: |
| `gltf_pbr` | Ash / Bevy | 15 | 3.9705 | 0.5682 |
| `gltf_pbr` | Ash / wgpu | 15 | 2.9197 | 0.0419 |
| `gltf_pbr` | Bevy / wgpu | 16 | 0.9877 | 0.4934 |
| `mtoon` | Ash / Bevy | 13 | 0.7854 | 0.3568 |
| `mtoon` | Ash / wgpu | 14 | 0.1429 | 0.0740 |
| `mtoon` | Bevy / wgpu | 15 | 0.7892 | 0.3384 |

現行 join では、`gltf_pbr` と `mtoon` のどちらも additive fit が gain fit より近いです。したがって、次の renderer parity 作業では「一括 exposure/gain」より、MToon/PBR の ambient/fill/light accumulation、texture sample ownership、material surface ごとの期待値を分けて詰めます。

## 関連ドキュメント

- 詳細な現状メモ: [`render-parity-current-images.md`](render-parity-current-images.md)
- 既存の日本語比較ページ: [`rendering-result-current-comparison.ja.md`](rendering-result-current-comparison.ja.md)
- 広い履歴インデックス: [`render-parity-image-comparison.md`](render-parity-image-comparison.md)
- parity 全体の進捗: [`render-parity.md`](render-parity.md)

## 更新コマンド

```powershell
just render-parity-samples-ash-gated
just render-parity-transparent-generated-ash-gated
just render-parity-gltf-pbr-generated
just render-parity-seed-base-color-flat32-render-resolve-readback
just render-parity-seed-base-color-flat32-shading-model-residual-join
```

最終判断では PNG だけを見ず、対応する `.imqraw` レポートと audit JSON を確認します。
