# 現状レンダリング結果 画像比較ボード

更新日: 2026-06-22

このページは、現在ローカルにあるレンダリング結果を目視比較するための短い Markdown です。画像と raw 比較レポートは `.external-fixtures/` 配下の作業用 artifact で、リポジトリには含めません。基準画像は `three-vrm`、比較対象は Rust 側の `wgpu` / Bevy / Ash です。

数値判断は PNG ではなく `.imqraw` の direct raw 比較を優先します。PNG は差分の場所を人間が見るための補助です。

## 先に見る結論

| 観点 | 現状 |
| --- | --- |
| Alpha / 透明 | 主要な現状セットでは wgpu / Bevy / Ash とも alpha mismatch は 0。 |
| wgpu と Ash | かなり近く、backend transport 単体より material / texture sampling / ownership の残差が濃い。 |
| Bevy | alpha は一致。Seed-san focused diagnostic でも wgpu/Ash とかなり近い残差帯に入り、renderer 固有の sample-copy 問題より material / fill / light accumulation を分けて見る段階。 |
| glTF/PBR | generated guard は良好。実モデル Seed-san の `backpack_nm` はまだ重点確認対象。 |
| MToon | body / arm / plastic / eye / bake surface の局所差分が残っている。 |

## Current Seed-san 基準セット

Artifact:
[`../.external-fixtures/render-parity-ash-current-base-uv-rerun`](../.external-fixtures/render-parity-ash-current-base-uv-rerun)

HTML visual review:
[`visual-review.html`](../.external-fixtures/render-parity-ash-current-base-uv-rerun/visual-review.html)

| Renderer | `rgb-visible` PSNR | Gradient PSNR | Changed RGB pixels | Max channel delta | Alpha mismatches |
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

読み: alpha と raw readback format は現在の blocker ではありません。差分は material color、texture sampling、edge / owner behavior 周辺に集中しています。

Raw reports:

- [`Seed-san.wgpu-vs-three-vrm.imqraw-rust.json`](../.external-fixtures/render-parity-ash-current-base-uv-rerun/reports/Seed-san.wgpu-vs-three-vrm.imqraw-rust.json)
- [`Seed-san.bevy-vs-three-vrm.imqraw-rust.json`](../.external-fixtures/render-parity-ash-current-base-uv-rerun/reports/Seed-san.bevy-vs-three-vrm.imqraw-rust.json)
- [`Seed-san.ash-vs-three-vrm.imqraw-rust.json`](../.external-fixtures/render-parity-ash-current-base-uv-rerun/reports/Seed-san.ash-vs-three-vrm.imqraw-rust.json)

## 実サンプル sweep

Artifact:
[`../.external-fixtures/render-parity-samples-ash-gated-check`](../.external-fixtures/render-parity-samples-ash-gated-check)

| Fixture | wgpu PSNR | Bevy PSNR | Ash PSNR | 読み |
| --- | ---: | ---: | ---: | --- |
| `Seed-san.vrm` | 34.6538 | 34.1163 | 34.6391 | 実モデルの主な material / ownership target。 |
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

| Fixture | Metric | wgpu | Bevy | Ash | 読み |
| --- | --- | ---: | ---: | ---: | --- |
| `transparent-blend_vrm` | `rgb-visible` | 54.3997 | 56.8605 | 54.3997 | alpha parity は一致。max channel delta は 1。 |
| `gltf-pbr_vrm` | `rgb-interior1px` | 47.8937 | 47.4289 | 47.8937 | Seed-san 風 backpack swatch を含めて non-MToon glTF/PBR fallback を guard。 |

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

このセクションは default behavior の目標ではなく、残差の原因を切り分ける診断です。expanded post-resolve は「source ownership が与えられた場合にどこまで近づくか」を見るためのもので、blind に適用する修正ではありません。

| Set | wgpu gradient PSNR | Bevy gradient PSNR | Ash gradient PSNR | 用途 |
| --- | ---: | ---: | ---: | --- |
| Current readback | 30.6336 | 28.2142 | 35.8902 | 現在の source of truth。 |
| Expanded post-resolve diagnostic | 32.7302 | 32.4826 | 35.8901 | target-pixel coverage と routed-sample の診断。Bevy は UV0/UV1 gradient 経路で更新済み。 |
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

Expanded diagnostic summary:
[`Seed-san.render-resolve-expanded.summary.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.render-resolve-expanded.summary.md)

読み: expanded diagnostic は target coverage の確認には有効ですが、expected-vs-actual 診断では material / draw key ごとの方向差が残っています。`backpack_nm` などの glTF/PBR 側と、body / plastic / eye / bake などの MToon 側を分けて追うのが次の安全な進め方です。これは「数値を見て opt-in knob を増やす」話ではなく、three-vrm と Rust 側の material evaluation がどの surface でどう違うかを固定 probe で切り分けるための読みです。

### 現行 join / summary の確認

実データの shading-model residual join は、Markdown と summary JSON の両方で additive / gain fit を保持しています。以前は summary parser の取り込み漏れで色 fit が空扱いになることがありましたが、現行 artifact では `color_fit` と `material_draw_color_fits` が JSON に入っています。`summary.json` 側に `"color_fit": null` が残らないことも、parser self-test と実 artifact の grep で確認済みです。

| Source | Path | 現状 |
| --- | --- | --- |
| Join Markdown | [`../target/texture-draw-audit/Seed-san.shading-model-residual-join.md`](../target/texture-draw-audit/Seed-san.shading-model-residual-join.md) | `Backend Color Fit` と `Material / Draw Color Fit` を含む。 |
| Join JSON | [`../target/texture-draw-audit/Seed-san.shading-model-residual-join.json`](../target/texture-draw-audit/Seed-san.shading-model-residual-join.json) | backend ごとの `color_fit` と draw-key ごとの `material_draw_color_fits` を含む。 |
| Expanded summary Markdown | [`../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.render-resolve-expanded.summary.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.render-resolve-expanded.summary.md) | join の色 fit 表を埋め込み済み。 |
| Expanded summary JSON | [`../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.render-resolve-expanded.summary.json`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-expanded-readback/reports/Seed-san.render-resolve-expanded.summary.json) | `preferred_fit` / `additive_rgb_delta` / `gain_fit_mean_distance` を保持。 |

現行の実データでは `gltf_pbr` の top residual は `backpack_nm node145/mesh4/prim9/base` に集中し、三 renderer とも draw-key 単位で additive fit が優勢です。MToon は `eye node2/mesh2/prim1/base` が additive、`arm_mat node144/mesh3/prim0/base` は Bevy / wgpu で gain が僅差優勢です。

| Track | Renderer | Rows | Mean E-A | Preferred | Additive RGB | Additive error | Gain error | 読み |
| --- | --- | ---: | ---: | --- | ---: | ---: | ---: | --- |
| `gltf_pbr/backpack_nm` | Ash | 16 | 32.6468 | additive | 17.06,19.12,20.19 | 5.4811 | 6.6892 | backpack/PBR fill の共通残差。 |
| `gltf_pbr/backpack_nm` | Bevy | 13 | 34.9706 | additive | 18.23,20.46,21.69 | 3.9750 | 6.6061 | wgpu と近く、Bevy 固有ではない。 |
| `gltf_pbr/backpack_nm` | wgpu | 12 | 35.6736 | additive | 18.50,20.83,22.25 | 3.7809 | 6.5428 | Ash/Bevy と同じ方向。 |
| `mtoon/eye` | Ash | 7 | 56.5272 | additive | 29.71,32.71,35.14 | 14.6302 | 22.4524 | eye surface の局所 fill 差。 |
| `mtoon/eye` | Bevy | 6 | 62.1792 | additive | 32.17,36.67,38.50 | 9.6965 | 22.6339 | wgpu と同じ方向。 |
| `mtoon/eye` | wgpu | 6 | 62.1882 | additive | 32.33,36.17,38.83 | 9.8403 | 22.7999 | eye は additive track。 |
| `mtoon/arm_mat` | Ash | 6 | 32.0326 | additive | 16.83,18.00,20.33 | 7.5106 | 8.1720 | Ash は additive が僅差。 |
| `mtoon/arm_mat` | Bevy | 4 | 36.7731 | gain | 20.00,20.75,22.75 | 5.4526 | 4.1067 | gain 優勢だが局所 track。 |
| `mtoon/arm_mat` | wgpu | 3 | 39.1200 | gain | 22.00,22.00,23.67 | 4.6623 | 4.1018 | gain 優勢だが一括 exposure ではない。 |

Material input report:
[`Seed-san.material-track-inputs.md`](../.external-fixtures/render-parity-seed-base-color-flat32-render-resolve-readback/reports/Seed-san.material-track-inputs.md)

この report は `just inspect-seed-material-tracks` で生成します。現行 blocker の入力は次のように分かれます。

| Track | Source material inputs | 次に見る shader path |
| --- | --- | --- |
| `gltf_pbr/backpack_nm` | material `14`、`gltf_pbr`、base texture `backpack`、normal texture `nm_backpack_normals`、roughness `0.657`、metallic `0`、occlusion/emissive なし。 | PBR fallback の normal/roughness/direct+ambient と edge-local owner sample。 |
| `mtoon/eye` | MToon + `KHR_materials_unlit`、base/shade texture は `faceparts`、shade color `[0.4352691, 0.3970382, 0.500747442]`、shift `-0.2`、toony `0.8`、GI `0.9`、rim/matcap/emissive なし。 | MToon shade multiply と direct/indirect diffuse。 |
| `mtoon/arm_mat` | MToon + `KHR_materials_unlit`、base/shade texture は `robo_arm`、shade color `[0.4352691, 0.3970382, 0.500747442]`、shift `-0.1`、toony `0.9`、GI `0.9`、parametric rim `0.07896994`、world outline `0.0015`。 | MToon shade/rim contribution と local fill/outline interaction。 |

## 詳細リンク

- 詳細な現状メモ: [`render-parity-current-images.md`](render-parity-current-images.md)
- 日本語の詳細比較: [`rendering-result-current-comparison.ja.md`](rendering-result-current-comparison.ja.md)
- 広い履歴インデックス: [`render-parity-image-comparison.md`](render-parity-image-comparison.md)
- parity 全体の進捗: [`render-parity.md`](render-parity.md)

## 更新コマンド

```powershell
just render-parity-samples-ash-gated
just render-parity-transparent-generated-ash-gated
just render-parity-gltf-pbr-generated
just render-parity-seed-base-color-flat32-render-resolve-readback
```

最終判断では PNG だけを見ず、対応する `.imqraw` レポートを確認します。
