# 多对多与 Join 实体

多对多关系在数据库层面需要一张**中间表（Join 实体）**。`rust-ef` 支持两种声明方式。

## 方式一：显式 Join 实体 + 泛型参数

```rust
#[derive(Debug, Clone, EntityType)]
#[table("students")]
pub struct Student {
    #[primary_key]
    #[auto_increment]
    pub student_id: i32,
    pub name: String,

    #[navigation]
    pub courses: HasMany<Course, Enrollment>,
}

#[derive(Debug, Clone, EntityType)]
#[table("courses")]
pub struct Course {
    #[primary_key]
    #[auto_increment]
    pub course_id: i32,
    pub title: String,
}

// Join 实体
#[derive(Debug, Clone, EntityType)]
#[table("enrollments")]
pub struct Enrollment {
    #[primary_key]
    #[auto_increment]
    pub enrollment_id: i32,

    #[foreign_key(Student)]
    pub student_id: i32,

    #[foreign_key(Course)]
    pub course_id: i32,
}
```

`HasMany<Course, Enrollment>` 告诉框架：加载 `Student.courses` 时，通过 `Enrollment` 表进行关联查询。

## 方式二：`#[through]` 属性

```rust
#[derive(Debug, Clone, EntityType)]
#[table("students")]
pub struct Student {
    #[primary_key]
    #[auto_increment]
    pub student_id: i32,
    pub name: String,

    #[navigation]
    #[through(Enrollment)]
    pub courses: HasMany<Course>,
}
```

两种方式功能等价，选择团队更喜欢的风格即可。

## 查询多对多

```rust
// 通过 linq! 的 include 子句预加载多对多导航
let students = linq!(ctx.set::<Student>(); include s.courses)
    .to_list()
    .await?;

// courses 已通过双查询策略自动物化
assert!(students[0].courses.len() > 0);
```

## 按主键查找

```rust
// 单主键：find 使用实体 PK 元数据，不再硬编码 "id"
let student = ctx.set::<Student>().query().find(1).await?;

// 复合主键：find_by_key 接收列名常量 + 值数组
use rust_ef::provider::DbValue;

let enrollment = ctx
    .set::<Enrollment>()
    .query()
    .find_by_key(&[
        (Enrollment::COLUMN_STUDENT_ID, DbValue::I32(1)),
        (Enrollment::COLUMN_COURSE_ID, DbValue::I32(2)),
    ])
    .await?;
```

## 设计要点

| 实践 | 说明 |
|------|------|
| Join 实体也需 `#[derive(EntityType)]` | 它需要有自己的表映射和主键 |
| Join 实体主键建议独立 | 不要只用 `student_id + course_id` 作为复合主键，便于后续扩展（如加入 `enrolled_at`） |

下一节：[Eager Loading：Include 与 ThenInclude](eager-loading.md)
