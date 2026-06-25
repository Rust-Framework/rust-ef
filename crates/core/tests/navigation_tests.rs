#[cfg(test)]
mod navigation_tests {
    use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
    use rust_ef::prelude::*;
    use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

    #[derive(Debug, Clone, EntityType)]
    #[table("nav_blogs")]
    struct NavBlog {
        #[primary_key]
        #[auto_increment]
        blog_id: i32,
        #[required]
        url: String,
        #[navigation]
        posts: HasMany<NavPost>,
    }

    #[derive(Debug, Clone, EntityType)]
    #[table("nav_posts")]
    struct NavPost {
        #[primary_key]
        #[auto_increment]
        post_id: i32,
        #[required]
        title: String,
        #[foreign_key(NavBlog)]
        blog_id: i32,
        #[navigation]
        blog: BelongsTo<NavBlog>,
        #[navigation]
        comments: HasMany<NavComment>,
    }

    #[derive(Debug, Clone, EntityType)]
    #[table("nav_comments")]
    struct NavComment {
        #[primary_key]
        #[auto_increment]
        comment_id: i32,
        #[required]
        text: String,
        #[foreign_key(NavPost)]
        post_id: i32,
    }

    #[tokio::test]
    async fn test_include_has_many_and_belongs_to() {
        let mut builder = DbContextOptionsBuilder::new();
        builder.use_sqlite_in_memory();
        let options = builder.build();
        let mut ctx = DbContext::from_options(&options).unwrap();

        ctx.set::<NavBlog>();
        ctx.set::<NavPost>();
        ctx.ensure_created().await.unwrap();

        ctx.set::<NavBlog>().add(NavBlog {
            blog_id: 0,
            url: "https://test.example".into(),
            posts: HasMany::new(),
        });
        ctx.save_changes().await.unwrap();

        let blogs = ctx.set::<NavBlog>().query().to_list().await.unwrap();
        let blog_id = blogs[0].blog_id;

        ctx.set::<NavPost>().add(NavPost {
            post_id: 0,
            title: "First".into(),
            blog_id,
            blog: BelongsTo::new(),
            comments: HasMany::new(),
        });
        ctx.set::<NavPost>().add(NavPost {
            post_id: 0,
            title: "Second".into(),
            blog_id,
            blog: BelongsTo::new(),
            comments: HasMany::new(),
        });
        ctx.save_changes().await.unwrap();

        let blogs = ctx
            .set::<NavBlog>()
            .query()
            .include_named("posts")
            .to_list()
            .await
            .unwrap();
        assert_eq!(blogs.len(), 1);
        assert_eq!(blogs[0].posts.len(), 2);

        let posts = ctx
            .set::<NavPost>()
            .query()
            .include_named("blog")
            .to_list()
            .await
            .unwrap();
        assert_eq!(posts.len(), 2);
        assert!(posts[0].blog.get().is_some());
        assert_eq!(posts[0].blog.get().unwrap().url, "https://test.example");
    }

    #[tokio::test]
    async fn test_then_include_nested() {
        let mut builder = DbContextOptionsBuilder::new();
        builder.use_sqlite_in_memory();
        let options = builder.build();
        let mut ctx = DbContext::from_options(&options).unwrap();

        ctx.set::<NavBlog>();
        ctx.set::<NavPost>();
        ctx.set::<NavComment>();
        ctx.ensure_created().await.unwrap();

        ctx.set::<NavBlog>().add(NavBlog {
            blog_id: 0,
            url: "https://nested.example".into(),
            posts: HasMany::new(),
        });
        ctx.save_changes().await.unwrap();
        let blog_id = ctx.set::<NavBlog>().query().to_list().await.unwrap()[0].blog_id;

        ctx.set::<NavPost>().add(NavPost {
            post_id: 0,
            title: "Post A".into(),
            blog_id,
            blog: BelongsTo::new(),
            comments: HasMany::new(),
        });
        ctx.save_changes().await.unwrap();
        let post_id = ctx.set::<NavPost>().query().to_list().await.unwrap()[0].post_id;

        ctx.set::<NavComment>().add(NavComment {
            comment_id: 0,
            text: "Great post".into(),
            post_id,
        });
        ctx.set::<NavComment>().add(NavComment {
            comment_id: 0,
            text: "Thanks".into(),
            post_id,
        });
        ctx.save_changes().await.unwrap();

        let blogs = ctx
            .set::<NavBlog>()
            .query()
            .include_named("posts")
            .then_include_named("comments")
            .to_list()
            .await
            .unwrap();

        assert_eq!(blogs.len(), 1);
        assert_eq!(blogs[0].posts.len(), 1);
        assert_eq!(blogs[0].posts.items()[0].comments.len(), 2);
        assert_eq!(blogs[0].posts.items()[0].comments.items()[0].text, "Great post");
    }
}
