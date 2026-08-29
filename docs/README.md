# Rust Entity Framework (rust-ef) 文档

本目录为 **Rust Entity Framework (rust-ef) 开发者手册：EFCore 风格的 Rust ORM** 的 canonical 源，面向从入门到生产的 ORM 实践。

## 结构

```
docs/
└── rust-ef/             # 文档根，对应 /api/docs/rust-ef/*
    ├── FOREWORD.md       # 前言
    ├── INDEX.md          # 全书目录
    ├── INDEX.json        # 文档网站左侧菜单
    ├── book.toml         # mdBook 配置
    ├── SUMMARY.md        # mdBook 目录
    └── 01-introduction/  # 章节（共 11 章）
        ├── 02-quickstart/           # 快速上手
        ├── 03-entity-design/        # 实体设计
        ├── 04-relationships/        # 关系与导航
        ├── 05-query-patterns/       # 查询模式
        ├── 06-advanced-query/       # 高级查询
        ├── 07-change-tracking/      # 变更跟踪
        ├── 08-bulk-operations/      # 批量操作
        ├── 09-transactions-migrations/  # 事务与迁移
        ├── 10-di-interceptors/      # DI 与拦截器
        └── 11-best-practices/       # 最佳实践与避坑
```

## 阅读

- [前言](rust-ef/FOREWORD.md) — 了解本书定位、读者画像与阅读路径
- [全书目录](rust-ef/INDEX.md)

## 阅读建议

- **快速上手**：从[第 2 章 快速上手](rust-ef/02-quickstart/)开始，掌握实体定义、DbContext 与第一个 CRUD
- **查询模式**：重点阅读[第 5 章 查询模式](rust-ef/05-query-patterns/)，涵盖 DbSet / IQueryable 与 `linq!` 宏
- **最佳实践**：参考[第 11 章 最佳实践与避坑](rust-ef/11-best-practices/)，含性能优化、安全与代码审查清单

## 迁移指南

- [v1.5 语义化版本迁移指南](v1.5-semver-migration-guide.md)
- [v1.8 迁移指南](v1.8-migration-guide.md)

## 维护

编辑 `docs/rust-ef/` 下的 Markdown 即可；Docbit 启动时会自动确保 `INDEX.json` 存在。