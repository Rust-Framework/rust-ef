#[cfg(test)]
mod m2m_tests {
    use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
    use rust_ef::prelude::*;
    use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

    #[derive(Debug, Clone, EntityType)]
    #[table("m2m_students")]
    struct Student {
        #[primary_key]
        #[auto_increment]
        student_id: i32,
        #[required]
        name: String,
        #[navigation]
        courses: HasMany<Course, Enrollment>,
    }

    #[derive(Debug, Clone, EntityType)]
    #[table("m2m_students_through")]
    struct StudentThrough {
        #[primary_key]
        #[auto_increment]
        student_id: i32,
        #[required]
        name: String,
        #[navigation]
        #[through(EnrollmentThrough)]
        courses: HasMany<Course>,
    }

    #[derive(Debug, Clone, EntityType)]
    #[table("m2m_courses")]
    struct Course {
        #[primary_key]
        #[auto_increment]
        course_id: i32,
        #[required]
        title: String,
    }

    #[derive(Debug, Clone, EntityType)]
    #[table("m2m_enrollments")]
    struct Enrollment {
        #[primary_key]
        #[auto_increment]
        enrollment_id: i32,
        #[foreign_key(Student)]
        student_id: i32,
        #[foreign_key(Course)]
        course_id: i32,
    }

    #[derive(Debug, Clone, EntityType)]
    #[table("m2m_enrollments_through")]
    struct EnrollmentThrough {
        #[primary_key]
        #[auto_increment]
        enrollment_id: i32,
        #[foreign_key(StudentThrough)]
        student_id: i32,
        #[foreign_key(Course)]
        course_id: i32,
    }

    async fn seed_m2m(ctx: &mut DbContext, use_through_attr: bool) {
        if use_through_attr {
            ctx.set::<StudentThrough>();
            ctx.set::<EnrollmentThrough>();
        } else {
            ctx.set::<Student>();
            ctx.set::<Enrollment>();
        }
        ctx.set::<Course>();
        ctx.ensure_created().await.unwrap();

        if use_through_attr {
            ctx.set::<StudentThrough>().add(StudentThrough {
                student_id: 0,
                name: "Alice".into(),
                courses: HasMany::new(),
            });
        } else {
            ctx.set::<Student>().add(Student {
                student_id: 0,
                name: "Alice".into(),
                courses: HasMany::new(),
            });
        }
        ctx.save_changes().await.unwrap();

        let student_id = if use_through_attr {
            ctx.set::<StudentThrough>()
                .query()
                .to_list()
                .await
                .unwrap()[0]
                .student_id
        } else {
            ctx.set::<Student>().query().to_list().await.unwrap()[0].student_id
        };

        ctx.set::<Course>().add(Course {
            course_id: 0,
            title: "Rust 101".into(),
        });
        ctx.set::<Course>().add(Course {
            course_id: 0,
            title: "ORM Design".into(),
        });
        ctx.save_changes().await.unwrap();
        let courses = ctx.set::<Course>().query().to_list().await.unwrap();

        if use_through_attr {
            ctx.set::<EnrollmentThrough>().add(EnrollmentThrough {
                enrollment_id: 0,
                student_id,
                course_id: courses[0].course_id,
            });
            ctx.set::<EnrollmentThrough>().add(EnrollmentThrough {
                enrollment_id: 0,
                student_id,
                course_id: courses[1].course_id,
            });
        } else {
            ctx.set::<Enrollment>().add(Enrollment {
                enrollment_id: 0,
                student_id,
                course_id: courses[0].course_id,
            });
            ctx.set::<Enrollment>().add(Enrollment {
                enrollment_id: 0,
                student_id,
                course_id: courses[1].course_id,
            });
        }
        ctx.save_changes().await.unwrap();
    }

    fn make_ctx() -> DbContext {
        let mut builder = DbContextOptionsBuilder::new();
        builder.use_sqlite_in_memory();
        let options = builder.build();
        DbContext::from_options(&options).unwrap()
    }

    #[tokio::test]
    async fn test_many_to_many_has_many_join_generic() {
        let mut ctx = make_ctx();
        seed_m2m(&mut ctx, false).await;

        let students = ctx
            .set::<Student>()
            .query()
            .include_named("courses")
            .to_list()
            .await
            .unwrap();

        assert_eq!(students.len(), 1);
        assert_eq!(students[0].courses.len(), 2);
    }

    #[tokio::test]
    async fn test_many_to_many_through_attribute() {
        let mut ctx = make_ctx();
        seed_m2m(&mut ctx, true).await;

        let students = ctx
            .set::<StudentThrough>()
            .query()
            .include_named("courses")
            .to_list()
            .await
            .unwrap();

        assert_eq!(students.len(), 1);
        assert_eq!(students[0].courses.len(), 2);
        let titles: Vec<&str> = students[0]
            .courses
            .items()
            .iter()
            .map(|c| c.title.as_str())
            .collect();
        assert!(titles.contains(&"Rust 101"));
        assert!(titles.contains(&"ORM Design"));
    }
}
