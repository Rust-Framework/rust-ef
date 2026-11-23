# rust-ef 发布清单（Release Checklist）

本文件是发布 `rust-ef` crate 家族（`rust-ef-macros` → `rust-ef` → 三个 provider）的
操作手册。发布由 tag 触发 `.github/workflows/publish.yml`，该工作流包含 preflight
（与 CI 的 lint/docs/semver 门禁等价）、安全审计、三方言 provider smoke、依赖顺序发布
与失败重试/幂等跳过。

## 0. 发布顺序与跨仓配对

```
rust-ef-macros  →  rust-ef  →  rust-ef-sqlite / rust-ef-postgres / rust-ef-mysql
```

依赖方必须先于被依赖方发布；`rust-ef-cli` 与 examples 设置 `publish = false`，不发布。

**跨仓顺序：先 rust-ef 2.8.0，后 rust-webx 2.8.0。** rust-webx 工作区用
`rust-ef = "2.6"` 的 semver 范围（可解析到 2.8.x）以在 ORM 发布前保持
`Cargo.lock` 可解析，但下游只有在 2.8.0 已出现在 crates.io 之后才能完成发布构建；
反过来先发 rust-webx 会让它的 crates.io 制品依赖一个尚不存在的版本。
配对验证的用法与证据边界见 rust-webx 仓库的
`scripts/check-pairing-rust-ef.sh` 与 `docs/design/compatibility.md` §4（本地
rust-ef checkout 经 `[patch.crates-io]` 编译 + `rust-webx-ef` 测试；不等价于
crates.io 制品的下载验证）。

## 1. 发布前检查（本地或 CI 全绿）

```bash
# 工作区干净且与远端一致
git status --short          # 必须为空
git pull --ff-only origin main

# 版本一致
cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.publish != []) | "\(.name) \(.version)"'
# 期望：rust-ef-macros/rust-ef/rust-ef-sqlite/rust-ef-postgres/rust-ef-mysql 均为目标版本

# 依赖单一 rust-dix（2.8 基线：rust-dix 1.0.x，三 crate 同版本，cargo tree -i 只能有一份）
cargo tree -i rust-dix

# 代码与文档门禁（与 CI 的 lint/docs/semver job 等价）
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --features chrono,uuid,decimal -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --skip postgres --skip mysql
cargo check --workspace --locked
cargo audit                                  # 必须 0 vulnerabilities
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
mdbook build docs/rust-ef                    # 文档站可构建（mdBook 0.5.4）
# 公开 API 的 SemVer 门禁（CI job `semver`；baseline 为最近一个已发布 tag，
# 已文档化的 breaking 在 workspace Cargo.toml 的
# [workspace.metadata.cargo-semver-checks.lints] 中登记）
cargo semver-checks --package rust-ef --baseline-rev v2.6.0

# 打包/发布演练
cargo package -p rust-ef-macros              # 叶子 crate 完整 verify
cargo publish -p rust-ef-macros --dry-run    # 含 verify 构建；依赖方需发布后才可 dry-run
cargo package -p rust-ef --no-verify --list  # 确认 README.md / Cargo.toml 在包内
```

> 依赖方（`rust-ef` / providers）的完整 `cargo package` verify 需要其 2.8.0
> 依赖已存在于 crates.io；这由发布工作流按顺序执行 `cargo publish` 时完成。
> 另有独立消费工程演练覆盖严格 scope / owned 解析 / 建表 / 写入 / 查询 / `dispose()`。

## 2. CHANGELOG 与版本

- [ ] `CHANGELOG.md` 顶部有目标版本条目（Breaking / Changed / Added / Migration）
- [ ] 工作区 `Cargo.toml` 的 `version` 与所有内部依赖版本一致
- [ ] `rust-version`（当前 1.95）与实际构建工具链匹配
- [ ] `docs/rust-ef/INDEX.json` 与 README 中的版本号已更新
- [ ] 迁移指南（当前 `docs/v2.8-migration-guide.md`）已更新；与配对发布说明
      （rust-webx 2.8.0）无版本串冲突

## 3. 打 tag 并发布

```bash
git tag -a v2.8.0 -m "rust-ef 2.8.0"
git push origin v2.8.0        # 触发 Publish workflow
```

工作流要求仓库 secret `CARGO_REGISTRY_TOKEN`（crates.io API token）。

发布步骤（workflow 自动执行）：

1. **preflight**：tag/版本一致、fmt、clippy（default / chrono+uuid+decimal /
   all-features）、单一 rust-dix、rustdoc `-D warnings`、semver-checks（baseline
   `v2.6.0`）、mdBook 构建、全量测试（SQLite 路径）、打包文件检查、MSRV 1.95
2. **audit**：`cargo generate-lockfile` 后 RustSec 审计，必须 0 漏洞
3. **provider-smoke**：SQLite / PostgreSQL 16 / MySQL 8 的
   generate → apply → introspect；真库 job 另跑 `provider_parity_tests --ignored`
4. **publish**：按顺序发布；每个 crate 发布后等待 crates.io API 可见，再发布下一个；
   已存在的版本自动跳过（幂等）；失败重试 3 次

## 4. 发布后核验

```bash
# 所有 crate 均可见（crates.io API 需要自定义 User-Agent）
UA="rust-ef-release-check (https://github.com/Rust-Framework/rust-ef)"
for c in rust-ef-macros rust-ef rust-ef-sqlite rust-ef-postgres rust-ef-mysql; do
  curl -fsS -A "$UA" "https://crates.io/api/v1/crates/$c/2.8.0" > /dev/null && echo "$c ok"
done

# 干净环境消费
cargo check --manifest-path /path/to/fresh-project/Cargo.toml
```

- [ ] crates.io 页面 README 为当前版本（非旧的 `lref` 内容）
- [ ] docs.rs 构建成功（各 crate 页面）
- [ ] 如后续恢复发布 CLI：`cargo install rust-ef-cli` 可用（当前 2.x CLI 为 `publish = false`）
- [ ] GitHub Release / 文档站点（`docs.yml`）更新
- [ ] 配对仓库 rust-webx 2.8.0 随后发布，并用其
      `scripts/check-pairing-rust-ef.sh`（或可选 CI pairing job）确认
      `pairing verified`（证据边界见该脚本头部注释）

## 5. 回滚

- 已发布版本不可删除，只能 **yank**：
  ```bash
  cargo yank --version 2.8.0 <crate>
  ```
- 依赖方按 **发布逆序** yank（先用 patch 版本修复）；`rust-ef 2.6.x + rust-dix 1.0.x`
  是 2.8 之前的稳定组合。
- 如果只是工作流失败：修复后重新 push tag（幂等跳过已发布 crate），不要移动已推送的 tag；
  必要时发布 `2.8.1` 修正版。
