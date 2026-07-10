//! Cascade save tests: many-to-many insert and cascade delete of join rows.

mod common;

use common::*;
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

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
    ctx.add::<CascadeStudent>(student);
    ctx.save_changes().await.unwrap();

    let students = ctx.set::<CascadeStudent>().query().to_list().await.unwrap();
    assert_eq!(students.len(), 1);
    let student_id = students[0].student_id;
    assert!(student_id > 0, "Student PK should be backfilled");

    let courses = ctx.set::<CascadeCourse>().query().to_list().await.unwrap();
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
async fn cascade_delete_m2m_join_rows() {
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
        courses: HasMany::with(vec![CascadeCourse {
            course_id: 0,
            title: "Math".into(),
        }]),
    };
    ctx.add::<CascadeStudent>(student);
    ctx.save_changes().await.unwrap();

    // Mark student Deleted — M2M join rows should be deleted, course preserved
    ctx.remove_at::<CascadeStudent>(0).unwrap();
    ctx.save_changes().await.unwrap();

    let students = ctx.set::<CascadeStudent>().query().to_list().await.unwrap();
    assert!(students.is_empty(), "Student table should be empty");

    let enrollments = ctx
        .set::<CascadeEnrollment>()
        .query()
        .to_list()
        .await
        .unwrap();
    assert!(
        enrollments.is_empty(),
        "Enrollment join rows should be deleted"
    );

    let courses = ctx.set::<CascadeCourse>().query().to_list().await.unwrap();
    assert_eq!(
        courses.len(),
        1,
        "Course should be preserved (M2M only deletes join rows)"
    );
}
