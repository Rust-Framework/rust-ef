# rust-ef 2.2.0 生产就绪与发布就绪报告

> 版本：**2.2.0**（2026-11-12） · 基线：rust-webx 1.0 / rust-ef 2.2 / rust-dix 0.10 · MSRV **1.95**
>
> 本文记录 2.2.0 的发布就绪结论与证据。1.x 时代的历史规格已归档到
> [`archive/PRODUCTION_READINESS_SPEC_v1.6.md`](archive/PRODUCTION_READINESS_SPEC_v1.6.md)。

## 1. 结论

| 项 | 结论 |
|---|---|
| 发布就绪 | ✅ 通过：本地门禁全绿 + MySQL 实例集成测试 + 打包/发布演练 |
| 版本裁定 | **2.2.0**（minor；含破坏性变更，见 CHANGELOG `[2.2.0]` 与迁移指南） |
| 适用生产 | SQLite / PostgreSQL / MySQL 均可；约束与实体配置能力按各方言矩阵生效 |
| 回滚方式 | crates.io `yank` + 修复版 patch（已发布版本不可删除） |

## 2. 变更范围（三组）

1. **数据库级约束与迁移自动化**：复合/部分/表达式索引、复合外键、CHECK、列默认值、
   MySQL 表选项、快照导出、`DbContext::migrate(dir)`、按方言的原子迁移脚本。
2. **EFCore 实体配置对齐**（原计划随 2.3 发布，本次并入 2.2.0）：列类型/精度/定长/
   排序规则/注释、计算列、备用键、视图、无键实体、schema 限定表、`#[derive(DbEnum)]`、
   `INCLUDE` 列。
3. **可组合谓词与单一条件编译器（破坏性）**：`Predicate<T>`（`and` / `or` / `not` /
   `and_opt` / `or_opt` / `always` / `never`）、`QueryBuilder::filter(impl Into<Predicate<T>>)`、
   `linq!(|b: T| ...)` 为唯一条件形式、值↔占位符单一编码器、无值条件 fail-fast、
   空 `IN` 列表语义化。

逐条迁移方式见 [`v2.2-migration-guide.md`](v2.2-migration-guide.md)（第 1 部分约束、
第 2 部分实体配置、第 3 部分条件 API）。

## 3. 质量门禁与证据（2026-11-12）

| 门禁 | 命令 / 范围 | 结果 |
|---|---|---|
| 测试（SQLite 路径） | `cargo test --workspace --all-features -- --skip postgres --skip mysql` | **76 目标 / 444 passed / 0 failed** |
| 集成测试（MySQL） | `cargo test --workspace --features chrono,uuid,decimal --test mysql_crud_tests -- --test-threads=1` | **9 passed / 0 failed**（共享实例 `rust_ef_test` 库） |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 warning |
| 文档 | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features` | 0 warning |
| 格式 | `cargo fmt --all -- --check` | 通过 |
| MSRV | `cargo +1.95.0 check --workspace --all-targets --all-features` | 通过 |
| 安全审计 | `cargo audit` | **0 漏洞 / 0 警告**（285 个锁定依赖） |
| 依赖单一性 | `cargo tree -i rust-dix` | 仅 `0.10.0` 一份 |
| 打包（叶子） | `cargo package -p rust-ef-macros` | 通过（完整 verify） |
| 打包（依赖方） | `cargo package -p <crate> --no-verify --list` | 四个 crate 均含 `README.md` + `Cargo.toml` |
| 发布演练 | `cargo publish -p rust-ef-macros --dry-run` | 打包 + verify 通过（上传中止） |
| 版本一致性 | `cargo metadata --no-deps` | 5 个可发布 crate 均为 `2.2.0` |

## 4. 兼容性

- **SemVer**：2.2 为 minor，但含破坏性变更（条件构建 API、`EntityTypeMeta` /
  `SnapshotColumn` 新字段、`EntityTypeBuilder::has_index` 返回 `IndexBuilder`、
  MySQL `VARCHAR`/UUID 读取修复）。迁移指南为每一条给出替代写法。
- **MSRV**：Rust **1.95**；`rust-dix` 固定 **0.10**，四个 `rust-ef*` crate 必须同版本。
- **锁文件**：`Cargo.lock` 不入库（库 crate 惯例），消费方与 CI 各自解析；
  本地验证使用 285 个锁定依赖。

## 5. 已知限制与风险

| 项 | 说明 |
|---|---|
| PostgreSQL 集成测试 | 本地无实例，执行时跳过；由 CI 三库 matrix（`postgres:16`）覆盖 |
| MySQL 测试实例 | 共享实例**不支持 TLS**，集成测试走明文（仅测试数据）；生产连接必须用 `use_mysql_tls` |
| 文档站 | mdBook 由 CI（`docs.yml`）构建并部署到 GitHub Pages；仓库不提交 `book/` 产物 |
| 迁移脚本切分 | 支持引号、注释、PG `$tag$`；不支持嵌套块注释（SQL 标准亦不要求） |
| 部分索引谓词 | 子查询、非有限浮点、无值条件会 fail-fast（不生成非法/恒假 DDL） |

## 6. 发布步骤

```bash
# 1) 打 tag 并推送（main 与 tag）
git tag -a v2.2.0 -m "rust-ef 2.2.0"
git push origin main
git push origin v2.2.0

# 2) 按依赖顺序发布（tag 触发的 workflow 同样按此顺序，幂等跳过已存在版本）
cargo publish -p rust-ef-macros
cargo publish -p rust-ef
cargo publish -p rust-ef-sqlite
cargo publish -p rust-ef-postgres
cargo publish -p rust-ef-mysql

# 3) GitHub Release（标题与正文取 CHANGELOG [2.2.0]）
```

## 7. 发布后核验

- crates.io 上 5 个 crate 的 `2.2.0` 均可见（API 需自定义 User-Agent）；
- docs.rs 五个 crate 构建成功；
- GitHub Release 附件/tag 与 `v2.2.0` 一致；
- GitHub Pages 文档站由 `docs.yml` 自动更新；
- 干净工程 `cargo check` 引入 `rust-ef = "2.2"` 成功。

## 8. 回滚

- 已发布版本不可删除，只能 yank：`cargo yank --version 2.2.0 <crate>`（按发布逆序）；
- 功能性问题以 `2.2.1` patch 修复；工作流失败时重推 tag（幂等跳过已发布 crate），
  不要移动已推送的 tag。
