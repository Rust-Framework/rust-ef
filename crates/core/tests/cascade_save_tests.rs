#[cfg(test)]
mod cascade_save_tests {
    use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
    use rust_ef::prelude::*;
    use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

    // ── One-to-many entities ──
    #[derive(Debug, Clone, EntityType)]
    #[table("cascade_blogs")]
    struct CascadeBlog {
        #[primary_key]
        #[auto_increment]
        blog_id: i32,
        #[required]
        url: String,
        #[navigation]
        posts: HasMany<CascadePost>,
    }

    #[derive(Debug, Clone, EntityType)]
    #[table("cascade_posts")]
    struct CascadePost {
        #[primary_key]
        #[auto_increment]
        post_id: i32,
        #[required]
        title: String,
        #[foreign_key(CascadeBlog)]
        blog_id: i32,
        #[navigation]
        blog: BelongsTo<CascadeBlog>,
    }

    // ── Self-referential entity ──
    #[derive(Debug, Clone, EntityType)]
    #[table("cascade_categories")]
    struct CascadeCategory {
        #[primary_key]
        #[auto_increment]
        category_id: i32,
        #[required]
        name: String,
        #[foreign_key(CascadeCategory)]
        parent_id: i32,
        #[navigation]
        children: HasMany<CascadeCategory>,
    }

    // ── Many-to-many entities ──
    #[derive(Debug, Clone, EntityType)]
    #[table("cascade_students")]
    struct CascadeStudent {
        #[primary_key]
        #[auto_increment]
        student_id: i32,
        #[required]
        name: String,
        #[navigation]
        courses: HasMany<CascadeCourse, CascadeEnrollment>,
    }

    #[derive(Debug, Clone, EntityType)]
    #[table("cascade_courses")]
    struct CascadeCourse {
        #[primary_key]
        #[auto_increment]
        course_id: i32,
        #[required]
        title: String,
    }

    #[derive(Debug, Clone, EntityType)]
    #[table("cascade_enrollments")]
    struct CascadeEnrollment {
        #[primary_key]
        #[auto_increment]
        enrollment_id: i32,
        #[foreign_key(CascadeStudent)]
        student_id: i32,
        #[foreign_key(CascadeCourse)]
        course_id: i32,
    }

    #[tokio::test]
    async fn cascade_insert_blog_with_posts() {
        let mut builder = DbContextOptionsBuilder::new();
        builder.use_sqlite_in_memory();
        let options = builder.build();
        let mut ctx = DbContext::from_options(&options).unwrap();

        ctx.set::<CascadeBlog>();
        ctx.set::<CascadePost>();
        ctx.ensure_created().await.unwrap();

        let blog = CascadeBlog {
            blog_id: 0,
            url: "https://cascade.example".into(),
            posts: HasMany::with(vec![
                CascadePost {
                    post_id: 0,
                    title: "First Post".into(),
                    blog_id: 0,
                    blog: BelongsTo::new(),
                },
                CascadePost {
                    post_id: 0,
                    title: "Second Post".into(),
                    blog_id: 0,
                    blog: BelongsTo::new(),
                },
            ]),
        };
        ctx.set::<CascadeBlog>().add(blog);
        ctx.save_changes().await.unwrap();

        let blogs = ctx.set::<CascadeBlog>().query().to_list().await.unwrap();
        assert_eq!(blogs.len(), 1);
        let blog_id = blogs[0].blog_id;
        assert!(blog_id > 0, "Blog PK should be backfilled");

        let posts = ctx.set::<CascadePost>().query().to_list().await.unwrap();
        assert_eq!(posts.len(), 2, "Two posts should be cascade-inserted");
        for post in &posts {
            assert_eq!(
                post.blog_id, blog_id,
                "Post blog_id should be fixed up to parent PK"
            );
        }
    }

    #[tokio::test]
    async fn cascade_insert_self_referential_tree() {
        let mut builder = DbContextOptionsBuilder::new();
        builder.use_sqlite_in_memory();
        let options = builder.build();
        let mut ctx = DbContext::from_options(&options).unwrap();

        ctx.set::<CascadeCategory>();
        ctx.ensure_created().await.unwrap();

        let root = CascadeCategory {
            category_id: 0,
            name: "Root".into(),
            parent_id: 0,
            children: HasMany::with(vec![
                CascadeCategory {
                    category_id: 0,
                    name: "Child A".into(),
                    parent_id: 0,
                    children: HasMany::new(),
                },
                CascadeCategory {
                    category_id: 0,
                    name: "Child B".into(),
                    parent_id: 0,
                    children: HasMany::new(),
                },
            ]),
        };
        ctx.set::<CascadeCategory>().add(root);
        ctx.save_changes().await.unwrap();

        let categories = ctx
            .set::<CascadeCategory>()
            .query()
            .to_list()
            .await
            .unwrap();
        assert_eq!(categories.len(), 3, "Root + 2 children");

        let root = categories
            .iter()
            .find(|c| c.name == "Root")
            .expect("Root should exist");
        assert!(root.category_id > 0, "Root PK should be backfilled");

        let children: Vec<&CascadeCategory> = categories
            .iter()
            .filter(|c| c.name == "Child A" || c.name == "Child B")
            .collect();
        assert_eq!(children.len(), 2);
        for child in &children {
            assert_eq!(
                child.parent_id, root.category_id,
                "Child parent_id should be fixed up via self-ref UPDATE"
            );
        }
    }

    #[tokio::test]
    async fn cascade_insert_many_to_many() {
        let mut builder = DbContextOptionsBuilder::new();
        builder.use_sqlite_in_memory();
        let options = builder.build();
        let mut ctx = DbContext::from_options(&options).unwrap();

        ctx.set::<CascadeStudent>();
        ctx.set::<CascadeCourse>();
        ctx.set::<CascadeEnrollment>();
        ctx.ensure_created().await.unwrap();

        let student = CascadeStudent {
            student_id: 0,
            name: "Alice".into(),
            courses: HasMany::with(vec![
                CascadeCourse {
                    course_id: 0,
                    title: "Math".into(),
                },
                CascadeCourse {
                    course_id: 0,
                    title: "Physics".into(),
                },
            ]),
        };
        ctx.set::<CascadeStudent>().add(student);
        ctx.save_changes().await.unwrap();

        let students = ctx
            .set::<CascadeStudent>()
            .query()
            .to_list()
            .await
            .unwrap();
        assert_eq!(students.len(), 1);
        let student_id = students[0].student_id;
        assert!(student_id > 0, "Student PK should be backfilled");

        let courses = ctx
            .set::<CascadeCourse>()
            .query()
            .to_list()
            .await
            .unwrap();
        assert_eq!(courses.len(), 2, "Two courses should be cascade-inserted");
        for course in &courses {
            assert!(course.course_id > 0, "Course PK should be backfilled");
        }

        let enrollments = ctx
            .set::<CascadeEnrollment>()
            .query()
            .to_list()
            .await
            .unwrap();
        assert_eq!(enrollments.len(), 2, "Two enrollment join rows");
        for enr in &enrollments {
            assert_eq!(enr.student_id, student_id, "Enrollment student_id matches");
            assert!(enr.course_id > 0, "Enrollment course_id should be set");
        }
    }

    #[tokio::test]
    async fn cascade_update_ordering() {
        let mut builder = DbContextOptionsBuilder::new();
        builder.use_sqlite_in_memory();
        let options = builder.build();
        let mut ctx = DbContext::from_options(&options).unwrap();

        ctx.set::<CascadeBlog>();
        ctx.set::<CascadePost>();
        ctx.ensure_created().await.unwrap();

        // Insert blog with one post
        let blog = CascadeBlog {
            blog_id: 0,
            url: "https://original.example".into(),
            posts: HasMany::with(vec![CascadePost {
                post_id: 0,
                title: "Original Title".into(),
                blog_id: 0,
                blog: BelongsTo::new(),
            }]),
        };
        ctx.set::<CascadeBlog>().add(blog);
        ctx.save_changes().await.unwrap();

        // Query back and modify
        let mut blogs = ctx.set::<CascadeBlog>().query().to_list().await.unwrap();
        let blog_id = blogs[0].blog_id;
        blogs[0].url = "https://updated.example".into();
        ctx.set::<CascadeBlog>().update(blogs.into_iter().next().unwrap());

        let mut posts = ctx.set::<CascadePost>().query().to_list().await.unwrap();
        posts[0].title = "Updated Title".into();
        ctx.set::<CascadePost>().update(posts.into_iter().next().unwrap());

        ctx.save_changes().await.unwrap();

        // Verify updates
        let blogs = ctx.set::<CascadeBlog>().query().to_list().await.unwrap();
        assert_eq!(blogs[0].url, "https://updated.example");

        let posts = ctx.set::<CascadePost>().query().to_list().await.unwrap();
        assert_eq!(posts[0].title, "Updated Title");
        assert_eq!(posts[0].blog_id, blog_id);
    }

    #[tokio::test]
    async fn cascade_delete_reverse_order() {
        let mut builder = DbContextOptionsBuilder::new();
        builder.use_sqlite_in_memory();
        let options = builder.build();
        let mut ctx = DbContext::from_options(&options).unwrap();

        ctx.set::<CascadeBlog>();
        ctx.set::<CascadePost>();
        ctx.ensure_created().await.unwrap();

        // Insert blog with one post
        let blog = CascadeBlog {
            blog_id: 0,
            url: "https://delete.example".into(),
            posts: HasMany::with(vec![CascadePost {
                post_id: 0,
                title: "To Delete".into(),
                blog_id: 0,
                blog: BelongsTo::new(),
            }]),
        };
        ctx.set::<CascadeBlog>().add(blog);
        ctx.save_changes().await.unwrap();

        // Mark entries for deletion (Post first, then Blog — reverse topo order
        // is handled by the save pipeline)
        ctx.set::<CascadePost>().remove_at(0).unwrap();
        ctx.set::<CascadeBlog>().remove_at(0).unwrap();
        ctx.save_changes().await.unwrap();

        // Verify tables are empty
        let blogs = ctx.set::<CascadeBlog>().query().to_list().await.unwrap();
        assert!(blogs.is_empty(), "Blog table should be empty after delete");
        let posts = ctx.set::<CascadePost>().query().to_list().await.unwrap();
        assert!(posts.is_empty(), "Post table should be empty after delete");
    }

    #[tokio::test]
    async fn cascade_empty_has_many_noop() {
        let mut builder = DbContextOptionsBuilder::new();
        builder.use_sqlite_in_memory();
        let options = builder.build();
        let mut ctx = DbContext::from_options(&options).unwrap();

        ctx.set::<CascadeBlog>();
        ctx.set::<CascadePost>();
        ctx.ensure_created().await.unwrap();

        let blog = CascadeBlog {
            blog_id: 0,
            url: "https://empty.example".into(),
            posts: HasMany::new(),
        };
        ctx.set::<CascadeBlog>().add(blog);
        ctx.save_changes().await.unwrap();

        let blogs = ctx.set::<CascadeBlog>().query().to_list().await.unwrap();
        assert_eq!(blogs.len(), 1);
        assert!(blogs[0].blog_id > 0, "Blog PK should be backfilled");

        let posts = ctx.set::<CascadePost>().query().to_list().await.unwrap();
        assert!(posts.is_empty(), "No posts should exist");
    }
}
