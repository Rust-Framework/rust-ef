# rust-ef 发布清单（Release Checklist）

本文件是发布 `rust-ef` crate 家族（`rust-ef-macros` → `rust-ef` → 三个 provider）的
操作手册。发布由 tag 触发 `.github/workflows/publish.yml`，该工作流包含 preflight、
安全审计、依赖顺序发布与失败重试/幂等跳过。

## 0. 发布顺序

```
rust-ef-macros  →  rust-ef  →  rust-ef-sqlite / rust-ef-postgres / rust-ef-mysql
```

依赖方必须先于被依赖方发布；`rust-ef-cli` 与 examples 设置 `publish = false`，不发布。

## 1. 发布前检查（本地或 CI 全绿）

```bash
# 工作区干净且与远端一致
git status --short          # 必须为空
git pull --ff-only origin main

# 版本一致
cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.publish != []) | "\(.name) \(.version)"'
# 期望：rust-ef-macros/rust-ef/rust-ef-sqlite/rust-ef-postgres/rust-ef-mysql 均为目标版本

# 依赖单一 rust-dix（2.1 要求 0.10.x）
cargo tree -i rust-dix

# 代码与文档门禁
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --skip postgres --skip mysql
cargo audit                                  # 必须 0 vulnerabilities
cargo doc --workspace --no-deps              # 无 warning
mdbook build docs/rust-ef                    # 文档站可构建

# 打包/发布演练
cargo package -p rust-ef-macros              # 叶子 crate 完整 verify
cargo publish -p rust-ef-macros --dry-run    # 含 verify 构建；依赖方需发布后才可 dry-run
cargo package -p rust-ef --no-verify --list  # 确认 README.md / Cargo.toml 在包内
```

> 依赖方（`rust-ef` / providers）的完整 `cargo package` verify 需要其 2.2.0
> 依赖已存在于 crates.io；这由发布工作流按顺序执行 `cargo publish` 时完成。
> 另有独立消费工程演练覆盖严格 scope / owned 解析 / 建表 / 写入 / 查询 / `dispose()`。

## 2. CHANGELOG 与版本

- [ ] `CHANGELOG.md` 顶部有目标版本条目（Breaking / Changed / Added / Migration）
- [ ] 工作区 `Cargo.toml` 的 `version` 与所有内部依赖版本一致
- [ ] `rust-version`（当前 1.95）与实际构建工具链匹配
- [ ] `docs/rust-ef/INDEX.json` 与 README 中的版本号已更新
- [ ] 迁移指南（如 `docs/v2.2-migration-guide.md`）已更新

## 3. 打 tag 并发布

```bash
git tag -a v2.2.0 -m "rust-ef 2.2.0"
git push origin v2.2.0        # 触发 Publish workflow
```

工作流要求仓库 secret `CARGO_REGISTRY_TOKEN`（crates.io API token）。

发布步骤（workflow 自动执行）：

1. **preflight**：tag/版本一致、fmt、clippy、全量测试（SQLite 路径）、打包文件检查
2. **audit**：`cargo generate-lockfile` 后 RustSec 审计，必须 0 漏洞
3. **publish**：按顺序发布；每个 crate 发布后等待 crates.io API 可见，再发布下一个；
   已存在的版本自动跳过（幂等）；失败重试 3 次

## 4. 发布后核验

```bash
# 所有 crate 均可见（crates.io API 需要自定义 User-Agent）
UA="rust-ef-release-check (https://github.com/Rust-Framework/rust-ef)"
for c in rust-ef-macros rust-ef rust-ef-sqlite rust-ef-postgres rust-ef-mysql; do
  curl -fsS -A "$UA" "https://crates.io/api/v1/crates/$c/2.2.0" > /dev/null && echo "$c ok"
done

# 干净环境消费
cargo check --manifest-path /path/to/fresh-project/Cargo.toml
```

- [ ] crates.io 页面 README 为当前版本（非旧的 `lref` 内容）
- [ ] docs.rs 构建成功（各 crate 页面）
- [ ] 如后续恢复发布 CLI：`cargo install rust-ef-cli` 可用（当前 2.x CLI 为 `publish = false`）
- [ ] GitHub Release / 文档站点（`docs.yml`）更新

## 5. 回滚

- 已发布版本不可删除，只能 **yank**：
  ```bash
  cargo yank --version 2.2.0 <crate>
  ```
- 依赖方按 **发布逆序** yank（先用 patch 版本修复）；`rust-ef` 2.0.x + `rust-dix 0.7`
  是 2.1 之前的稳定组合。
- 如果只是工作流失败：修复后重新 push tag（幂等跳过已发布 crate），不要移动已推送的 tag；
  必要时发布 `2.1.1` 修正版。
