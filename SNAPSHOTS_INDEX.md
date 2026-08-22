# 目录结构快照索引（Step 1）

通过 GitHub API（`gh api` + `git/trees?recursive=1`，对超大仓库回退 BFS）获取，存放于 `snapshots/`。

## 单仓库
| 仓库 | 默认分支 | 条目数 | 快照文件 |
|---|---|---|---|
| cuberite/cuberite | master | 1481 | cuberite__cuberite.txt |
| pmh1314520/MCTier | master | 526 | pmh1314520__MCTier.txt |

## 组织 FCL-Team（24 个仓库，全部已抓取）
FCLRendererPlugin 因 GitHub **HTTP 451** 访问受限，仅保留占位说明。
其余：FoldCraftLauncher、FCLCore 等见 `FCL-Team__*.txt`（含 Holy-GL4ES、OpenAL、mesa、lwjgl3、lwjgl-fcl、caciocavallo-FCL、FCL-Controllers、Android-OpenJDK-Build、Android-Easytier-Build、zstd-jni-DH、NG-GL4ES、LWJGL-Pojav、EnchantNet、EnchantNetCore、FCLDriverPlugin、FCL-Repo、FCL-Docs、FCL-Team.github.io 等）。

## 组织 ZalithLauncher（15 个仓库，全部已抓取）
ZalithRendererPlugin 因 GitHub **HTTP 451** 访问受限，仅保留占位说明。
其余：ZalithLauncher、ZalithLauncher2、ZalithWebsite、ZalithJars、Zalith-Info、zalithdocs、SDL、LWJGL-AAMC、lwjgl3、NativeLibPlugin、OptiFineRenamer、RendererPlugin、RendererPlugin-v2、VerifiedPluginLoad 见 `ZalithLauncher__*.txt`。

## 统计
- 目标总数：41（2 单仓库 + FCL-Team 24 + ZalithLauncher 15）
- 成功快照：39
- 访问受限（HTTP 451）：2（FCL-Team/FCLRendererPlugin、ZalithLauncher/ZalithRendererPlugin）
