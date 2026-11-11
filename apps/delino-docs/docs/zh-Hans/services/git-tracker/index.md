# Git Tracker

在 [git-tracker.delino.io](https://git-tracker.delino.io) 跟踪 Git 提交的数值测量结果。Git Tracker 免费使用。它支持公开和私有 GitHub 仓库，包括来自 fork 的拉取请求。可用于文件大小、耗时、吞吐量、计数及其他标量测量。

## 连接仓库

1. 使用你的 Delino 账户登录，并选择个人或组织空间。
2. 请空间所有者通过空间的组授予 Git Tracker 查看、编辑或管理权限。查看权限允许读取。编辑权限额外允许上传和标记完成。管理权限额外允许连接、令牌管理和永久删除。
3. 为仓库安装 Git Tracker GitHub App。
4. 打开 **管理仓库**。输入 `owner/repository`，以及已关联 GitHub 账户的 GitHub 访问令牌，该账户须对仓库具有管理员访问权限。服务会验证你的 GitHub 权限，但不会保留该访问令牌。
5. 连接仓库。你还需要 Delino 空间所有者/管理员权限。一个仓库同一时间只能连接到一个空间。从仓库管理中复制其连接 ID，以用于 Action 和 API 调用。断开连接后，其他获得授权的空间可以使用新的连接 ID 连接该仓库；旧空间保留的历史记录仍为私有。

无需登录即可读取公开仓库的测量数据。私有测量数据要求具有空间成员资格和 Git Tracker 读取权限。空间访问权限适用于该空间中所有已连接的仓库。

断开连接或移除 App 会停止上传，并将保留的历史记录设为私有。获得授权的成员仍可读取这些记录。仓库从公开变为私有的变更经确认后，历史数据也会设为私有。如果 GitHub 无法确认当前可见性，Git Tracker 会保留上次确认的可见性，直到同步成功。

## 定义数据集

在仓库中保存一份 YAML 配置。ID 是稳定的；更改名称不会丢失历史记录。一行中的所有列必须一起上传。

```yaml
id: file-size
name: JavaScript file sizes
columns:
  - id: original
    name: Original size
    unit: B
    direction: lower
    absoluteThreshold: 1024
  - id: minified
    name: Minified size
    unit: B
    direction: lower
    percentageThreshold: 1
  - id: gzip
    name: Gzipped size
    unit: B
    direction: lower
    absoluteThreshold: 128
    percentageThreshold: 2
```

上传 JSON 行，例如 SWC 风格的文件大小表：

```json
{
  "rows": [
    { "id": "jquery", "name": "jQuery", "values": { "original": 285314, "minified": 87533, "gzip": 30321 } },
    { "id": "three", "name": "Three.js", "values": { "original": 1321000, "minified": 675000, "gzip": 170000 } }
  ]
}
```

Action 会保留 JSON 十进制数的原始写法，不会进行二进制浮点数转换。直接调用 Connect 的调用方应以字符串形式发送十进制值，例如 `"9007199254740993.0000000000000001"`。支持负数和有限小数。显示时的舍入绝不会改变退化判定。

支持的单位包括 `B`、`kB`、`MB`、`GB`、`KiB`、`MiB`、`GiB`；`ns`、`μs`、`ms`、`s`；`count`、`%`、`unitless`；以及 `ops/s`、`B/s`、`kB/s`、`MB/s`、`GB/s`、`KiB/s`、`MiB/s`、`GiB/s`。兼容的单位会在比较前进行换算。不兼容的类型无法比较。例如，1048576 B 显示为 1 MiB；表格中仍可查看精确值。

方向包括 `lower`、`higher` 和 `neutral`。中性列永远不会产生退化。阈值适用于该列的所有行。同时设置绝对阈值和百分比阈值时，超过任一阈值即为退化；等于阈值不算。未设置阈值时，会显示变化，但不会因退化而判定失败。仅对朝恶化方向的变化作出判定。

百分比变化的计算公式为 `(new - baseline) / abs(baseline) * 100`。基线为零时，不计算百分比变化。绝对阈值仍可用于判定；仅有百分比阈值的规则则暂不作出判定。比较使用目标执行所存储的配置，而非之后的配置。

## 使用官方 GitHub Action

将 `delinoio/apps/actions/git-tracker` 固定到经过审查的完整提交 SHA。将下方占位符替换为你的 Git Tracker 连接 ID、checkout 版本和 Action 版本。授予 `id-token: write` 权限以使用 OIDC。也可以使用仓库上传令牌；将其存储为 GitHub 密钥，并通过 `token` 传入。

```yaml
permissions:
  contents: read
  id-token: write
jobs:
  measure:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@<reviewed-checkout-commit-sha>
      - run: ./measure.sh
      - uses: delinoio/apps/actions/git-tracker@<reviewed-action-commit-sha>
        with:
          repository-id: '019b0000-0000-7000-8000-000000000001'
          configuration: measurements.yaml
          measurements: measurements.json
          fail-on-regression: 'false'
  mark-as-done:
    needs: measure
    if: always()
    runs-on: ubuntu-latest
    steps:
      - uses: delinoio/apps/actions/git-tracker@<reviewed-action-commit-sha>
        with:
          operation: complete
          repository-id: '019b0000-0000-7000-8000-000000000001'
```

独立作业共享默认执行 ID `repository_id/run_id/run_attempt`，并且可以上传互不重叠的行。重新运行时请使用新的执行 ID。Actions OIDC 要求使用默认标识；自定义执行 ID 供使用仓库令牌的调用方使用。对于 OIDC 凭据属于已连接仓库的拉取请求作业，可设置 Action 的 `baseline` 输入，以使用明确指定的完整提交 SHA。来自 fork 的 OIDC 上传仅限于 PR 头提交，不能覆盖基线，也不能写入上游推送历史。

每个上传作业仅判定自身上传的行。默认情况下会报告退化，但不会使 CI 失败。设置 `fail-on-regression: 'true'` 可在发生退化时使该作业失败。缺少比较结果不会使 CI 失败。完成作业仅表示测量已结束；它不会提交 CI 成功/失败/取消状态，也不会执行汇总门禁判定。所有作业都被跳过时，空的完成操作也有效。

网络错误和暂时性 API 故障最多重试三次。默认情况下，重试后仍未成功会使作业失败。设置 `warn-on-api-error: 'true'` 可改为发出警告并通过。PR 评论失败仅会发出警告；不会使已保存的数据失效，也不会单独导致 CI 失败。Action 会在日志中记录详情链接，并写入作业摘要。

一条 PR 评论会反映 PR 最新提交上最近注册的执行的累计结果。较早的运行仍可在网页历史记录中查看。

## 调用 Connect API

API 源地址为 `https://git-tracker.api.delino.io`。Connect JSON 请求使用 `Content-Type: application/json` 和 `Connect-Protocol-Version: 1`。管理者可以签发仓库令牌，必须设置未来的到期时间，并可在仓库管理中撤销这些令牌。

```sh
curl --fail-with-body \
  'https://git-tracker.api.delino.io/gittracker.v1.GitTrackerService/Upload' \
  -H "Authorization: Bearer ${GIT_TRACKER_TOKEN}" \
  -H 'Content-Type: application/json' \
  -H 'Connect-Protocol-Version: 1' \
  --data-binary @upload.json
```

`upload.json` 包含 `context`、`dataset` 和 `rows`：

```json
{
  "context": {
    "repositoryId": "019b0000-0000-7000-8000-000000000001",
    "executionId": "my-historical-run",
    "commit": "0123456789012345678901234567890123456789",
    "branch": "main"
  },
  "dataset": {
    "id": "size",
    "name": "Size",
    "columns": [{ "id": "bytes", "name": "Bytes", "unit": "B", "direction": "DIRECTION_LOWER", "absoluteThreshold": "1024" }]
  },
  "rows": [{ "id": "bundle", "name": "Bundle", "values": { "bytes": "1048576" } }]
}
```

使用相同的 `context` 调用 `/gittracker.v1.GitTrackerService/Complete`。对于 PR 上传，请包含 `pullRequest`，并使用 PR 的基分支和头提交。`baselineCommit` 用于明确指定完整的基线 SHA。历史提交使用相同的上传 API。发送前请勿通过 JavaScript `Number` 转换十进制字符串。

上传或完成操作发送的文本中不得包含 NUL（`\u0000`）字符。Git Tracker 会拒绝该请求，并返回指出受影响字段的客户端错误。其他受支持的 Unicode 文本和控制字符仍然有效。

## 执行和比较规则

首次上传会冻结该执行中每个数据集的配置。之后若配置不同，将被拒绝。以相同数值重复上传已有行具有幂等性。冲突的数值会被拒绝。批次具有原子性：如果某行无效或存在冲突，该请求中的任何数据都不会保存。

数据会立即以 **测量中** 状态显示。只有完成后，它才会成为代表性结果。一个提交的代表性结果是其最近注册且已完成的执行，包括存在退化的执行。较早的运行即使较晚完成，也不会替换注册时间较新的已完成运行。重复标记完成既不会改变完成时间，也不会改变排序。

如果连续六小时没有新保存的测量数据，未完成的执行会变为 **已中断**。相同内容的重传不会重置该计时器。已完成和已中断的执行会拒绝后续上传；已中断的执行也会拒绝完成操作。请改为启动新的执行。数据会一直保留，直到你将其删除。

自动 PR 比较会在首次上传时冻结共同祖先（merge-base）提交及其代表性已完成执行。缺失的基线即使之后上传，也仍不可用。自动推送比较会针对每一行分别检查最近的最多 1,000 个第一父级祖先，并使用在首次上传截止时间前已完成测量的最近祖先。如果该范围内没有符合条件的行测量数据，则无法进行比较。后续作业使用相同的截止时间。表格会标明每个实际来源提交和执行。你可以明确设置基线 SHA，以便与更早的提交进行比较；显式比较不会使用行级祖先回退机制。

网站的双提交比较会优先为每个所选提交使用最近注册且已完成的分支执行。如果某个提交没有已完成的分支执行，则使用该提交最近注册且已完成的拉取请求执行。所选目标执行提供其存储的数据集配置。新增行/列以及缺失或不兼容的数据无法比较，也不会导致退化判定失败。已删除的基线数据不会被重建。完成前，不存在的行视为尚未采集；完成后则视为缺失。

图表按第一父级顺序等间距排列提交。它们会跨过被跳过的提交连接实际测量点，绝不会沿用或虚构数值。可按数据集、分支、列、条目、日期和提交范围筛选。使用数据表查看精确值，以及可通过键盘访问的执行链接。

## 删除和故障排查

管理者可以在确认后永久删除仓库的测量数据、某个数据集或一次执行。删除未知执行或当前没有测量数据的数据集时，会报告未找到目标。删除数据集仅影响包含该数据集的执行；其他执行仍可上传该数据集。基线测量数据被删除后，相应比较将变为不可用；如果基线从未包含该数据集，则会将其视为新数据。当 Delcore 最终完成空间删除时，Git Tracker 会删除该空间的连接、令牌、配置和历史记录。

遇到权限错误时，请核对所选空间和你当前的 Git Tracker 角色。遇到上传冲突时，请确认执行标识、数据集配置和完整的行值。对于已中断或已完成的运行，请使用新的执行 ID。遇到网络/服务器错误时，请重试，并在寻求帮助时保留响应关联 ID。切勿在公开报告中分享令牌、凭据或私有测量数据。

PR 执行的测量数据仍可查看，但不会提供上游分支图表数据点或自动基线。这可防止 fork 的测量数据替换同一提交 SHA 在基线分支上的数据或配置。
