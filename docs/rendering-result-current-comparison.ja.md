# 現状レンダリング結果の画像比較

更新日: 2026-06-21

このページは、いまローカルに存在するレンダリング比較画像をそのまま並べるためのレビュー用Markdownです。画像・raw比較レポートは `.external-fixtures/` 配下にあり、リポジトリにはコミットしません。基準画像は `three-vrm`、比較対象は Rust 側の `wgpu` / Bevy / Ash です。

数値判断は direct raw 比較の `.imqraw` / `.psnr.json` を優先します。ここでは、目視確認しやすいように PNG を横並びにしています。

## 現時点の結論

この Markdown で最初に見るべきなのは、`Current Seed-san 基準セット` と `Seed-san blocker diagnostic` です。前者は「現状の全体像」、後者は「まだ three-vrm と一致していない原因候補」を見るための画像セットです。

| 観点 | 現状 |
| --- | --- |
| 透明 / alpha | wgpu / Bevy / Ash とも主要セットでは alpha mismatch は出ていません。 |
| wgpu と Ash | ほぼ同じ傾向で、backend transport より material/color/texture sampling 側が主な残差候補です。 |
| Bevy | alpha は一致していますが、Seed-san の focused diagnostic では selected sample に寄りすぎるケースがあり、material/color/fill の見方を wgpu/Ash と分ける必要があります。 |
| glTF/PBR | generated guard は良好ですが、Seed-san の `backpack_nm` はまだ three-vrm より暗い方向の残差が残っています。 |
| MToon | body / arm / plastic / eye / bake surface の局所差分が残っています。 |

直近の shading-model residual join では、`gltf_pbr` は `17` 個の共有 top-residual pixel がすべて `backpack_nm node145/mesh4/prim9/base` に集まっています。`mtoon` は `14` 個の共有 top-residual pixel があり、wgpu/Ash は近い一方、Bevy は `huku_bake` と `eye` の残差が目立ちます。

追加の shared backend 診断では、`gltf_pbr` の wgpu/Ash actual RGB distance は `2.92`、`mtoon` は `0.14` まで近く、Bevy は selected sample exact の比率が高いです。したがって次の実装は、wgpu/Ash 共通の material output と Bevy の selected-sample/fill 挙動を分けて追います。

## まず見る画像セット

現状の主要な比較対象は、次の順で見るのが分かりやすいです。

| 用途 | ディレクトリ | 見るポイント |
| --- | --- | --- |
| 現在の Seed-san 基準セット | [`../.external-fixtures/render-parity-ash-current-base-uv-rerun`](../.external-fixtures/render-parity-ash-current-base-uv-rerun) | three-vrm / wgpu / Bevy / Ash の全体像 |
| 実サンプル sweep | [`../.external-fixtures/render-parity-samples-ash-gated-check`](../.external-fixtures/render-parity-samples-ash-gated-check) | Seed-san、constraint、UV animation、expression、VRM0 |
| 透明 blend guard | [`../.external-fixtures/render-parity-transparent-generated-ash-gated`](../.external-fixtures/render-parity-transparent-generated-ash-gated) | alpha / blend の regression |
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

読み: このセットでは alpha / raw readback format は blocker ではありません。残差は material color、texture sampling、edge / owner behavior 周辺に寄っています。

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
| `VRMC_materials_mtoon_UV_Animation_Test.vrm` | 35.6342 | 35.6202 | 35.6342 | UV animation path はカバー済み。 |
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

直近の quad resolve 後の expanded readback は、wgpu / Bevy / Ash すべてで alpha は一致しています。Ash PNG は `just render-parity-current-ash-pngs` で既存の `.rgba.json` から byte-equivalent に補完できます。

| Set | wgpu gradient PSNR | Bevy gradient PSNR | Ash gradient PSNR | 用途 |
| --- | ---: | ---: | ---: | --- |
| Current readback | 30.6336 | 28.2142 | 35.8902 | 現在の source of truth。 |
| Expanded post-resolve diagnostic, current local artifact | 32.7302 | 32.4826 | 35.8901 | target-pixel coverage の診断。Bevy は UV0/UV1 gradient 経路で更新済み。 |
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

### Expanded readback の material/color 診断

`target/texture-draw-audit/` に出している expected-vs-actual (`E-A`) 診断では、selected bucket の mean E-A distance は wgpu `45.89`、Ash `36.97`、Bevy `93.25` です。方向は material / draw key ごとに分かれており、単一の exposure / color-space knob で直す形ではありません。

| Renderer | Selected mean E-A | 代表的な expected-brighter bucket | 代表的な expected-darker bucket | 読み |
| --- | ---: | --- | --- | --- |
| wgpu | 45.89 | `backpack_nm node145/mesh4/prim9/base` `+18.47,+21.00,+22.33` | `body_nm node145/mesh4/prim1/base` `-33.00,-26.50,-24.75` | glTF/PBR backpack と MToon body/plastic を分けて見る。 |
| Ash | 36.97 | `backpack_nm node145/mesh4/prim9/base` `+16.57,+18.83,+19.83` | `body_nm node145/mesh4/prim1/base` `-9.50,-7.88,-7.75` | wgpu と同じ方向の split。backend だけの差ではない。 |
| Bevy | 93.25 | `backpack_nm` 周辺が大きく expected-brighter | material pixel residual が大きい | manifest sample 追従とは別に material/color 評価差が残る。 |

Audit Markdown:

- [`target/texture-draw-audit/Seed-san.wgpu.expected-actual.md`](../target/texture-draw-audit/Seed-san.wgpu.expected-actual.md)
- [`target/texture-draw-audit/Seed-san.bevy.expected-actual.md`](../target/texture-draw-audit/Seed-san.bevy.expected-actual.md)
- [`target/texture-draw-audit/Seed-san.ash.expected-actual.md`](../target/texture-draw-audit/Seed-san.ash.expected-actual.md)
- [`target/texture-draw-audit/Seed-san.shading-model-residual-join.md`](../target/texture-draw-audit/Seed-san.shading-model-residual-join.md)

### Shading model 別の現在の残差

`just render-parity-seed-base-color-flat32-shading-model-residual-join` で、expanded texture/color audit の `top_residuals_by_shading_model` を wgpu / Bevy / Ash 横断で join できます。これは画像を再解釈する処理ではなく、既存 audit JSON の pixel probe を並べるための診断です。

| Shading model | 共有 top-residual pixels | wgpu mean E-A | Bevy mean E-A | Ash mean E-A | 主な surface / draw key | 読み |
| --- | ---: | ---: | ---: | ---: | --- | --- |
| `gltf_pbr` | 17 | 33.45 | 89.46 | 32.65 | `backpack_nm`, `node145/mesh4/prim9/base` | backpack/PBR の color accumulation を独立して追う。 |
| `mtoon` | 14 | 45.87 | 100.16 | 45.62 | `eye`, `arm_mat`, `arm_plastic`, `huku_bake` | MToon の body/plastic/eye/bake surface を分けて追う。 |

この表から見る限り、次の修正は「全体 exposure を一つ動かす」より、`gltf_pbr` の backpack 系と `mtoon` の局所 material/fill を別々に合わせ込む方が安全です。

現在の shared backend 診断では、`gltf_pbr` は wgpu/Ash がほぼ同じ出力で Bevy が selected sample に寄る傾向、`mtoon` は wgpu/Ash がさらに強く一致して Bevy の shared rows は selected sample exact です。これは「三 renderer 共通の一括色補正」ではなく、renderer group ごとの原因分離が必要という読みを補強します。

## 現時点の読み

- `wgpu` と Ash は多くのセットでほぼ同じ傾向を示しており、backend transport だけではなく material/color/texture sampling 側の差分が主な候補です。
- Bevy は alpha が一致しており、広域の透明処理 regression は現状見えていません。ただし Seed-san の current diagnostic では gradient-domain PSNR が低く、material pixel の residual は残っています。
- `transparent-blend_vrm` と `gltf-pbr_vrm` の guard は良好です。透明・非MToon fallback の大きな regression は、現状の画像では見えていません。
- quad resolve 後の expanded diagnostic は target coverage の改善確認としては有効ですが、E-A 診断では material / draw key ごとの方向差が残っています。次は全体ノブではなく、glTF/PBR backpack 系と MToon body/plastic 系を分けて詰めます。
- second-frontier diagnostic は修正結果として扱わず、root-cause を探すための negative control として使います。

## 関連ドキュメント

- 目視用の短い画像ボード: [`rendering-result-current-visual-board.ja.md`](rendering-result-current-visual-board.ja.md)
- 詳細な数値・診断履歴: [`render-parity-current-images.md`](render-parity-current-images.md)
- 広い履歴インデックス: [`render-parity-image-comparison.md`](render-parity-image-comparison.md)
- parity 全体の進捗: [`render-parity.md`](render-parity.md)
