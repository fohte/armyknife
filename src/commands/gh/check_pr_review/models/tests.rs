use super::comment::{Author, PullRequestReview, ReplyTo};
use super::thread::CommentsNode;
use super::*;

fn make_comment(id: i64, review_id: Option<i64>, is_reply: bool) -> Comment {
    Comment {
        database_id: id,
        author: Some(Author {
            login: "user".to_string(),
        }),
        body: "comment body".to_string(),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        path: Some("file.rs".to_string()),
        line: Some(10),
        original_line: None,
        diff_hunk: None,
        reply_to: if is_reply { Some(ReplyTo {}) } else { None },
        pull_request_review: review_id.map(|id| PullRequestReview { database_id: id }),
    }
}

fn make_thread(review_id: Option<i64>, is_resolved: bool) -> ReviewThread {
    ReviewThread {
        is_resolved,
        comments: CommentsNode {
            nodes: vec![make_comment(1, review_id, false)],
        },
    }
}

fn make_review(id: i64, body: &str, state: ReviewState) -> Review {
    Review {
        database_id: id,
        author: Some(Author {
            login: "reviewer".to_string(),
        }),
        body: body.to_string(),
        state,
        created_at: "2024-01-01T00:00:00Z".to_string(),
    }
}

#[test]
fn test_threads_for_review() {
    let pr_data = PrData {
        reviews: vec![
            make_review(100, "", ReviewState::Approved),
            make_review(200, "comment", ReviewState::Commented),
        ],
        threads: vec![
            make_thread(Some(100), false),
            make_thread(Some(100), true),
            make_thread(Some(200), false),
        ],
    };

    let threads_100 = pr_data.threads_for_review(100);
    assert_eq!(threads_100.len(), 2);

    let threads_200 = pr_data.threads_for_review(200);
    assert_eq!(threads_200.len(), 1);

    let threads_999 = pr_data.threads_for_review(999);
    assert!(threads_999.is_empty());
}

#[test]
fn test_orphan_threads() {
    let pr_data = PrData {
        reviews: vec![make_review(100, "", ReviewState::Approved)],
        threads: vec![
            make_thread(Some(100), false), // belongs to review 100
            make_thread(Some(999), false), // orphan (review 999 doesn't exist)
            make_thread(None, false),      // orphan (no review association)
        ],
    };

    let orphans = pr_data.orphan_threads();
    assert_eq!(orphans.len(), 2);
}

#[test]
fn test_empty_body_review_preserved() {
    let pr_data = PrData {
        reviews: vec![
            make_review(100, "", ReviewState::Approved),
            make_review(200, "has body", ReviewState::ChangesRequested),
        ],
        threads: vec![make_thread(Some(100), false)],
    };

    assert_eq!(pr_data.reviews.len(), 2);
    assert_eq!(pr_data.threads_for_review(100).len(), 1);
}

#[test]
fn test_count_unresolved() {
    let threads = [
        make_thread(Some(1), false),
        make_thread(Some(1), true),
        make_thread(Some(1), false),
    ];
    let refs: Vec<&ReviewThread> = threads.iter().collect();
    assert_eq!(ReviewThread::count_unresolved(&refs), 2);
}

#[test]
fn test_root_comment_and_replies() {
    let thread = ReviewThread {
        is_resolved: false,
        comments: CommentsNode {
            nodes: vec![
                make_comment(1, Some(100), false), // root
                make_comment(2, Some(100), true),  // reply
                make_comment(3, Some(100), true),  // another reply
                make_comment(4, Some(100), true),  // nested reply
            ],
        },
    };

    let root = thread.root_comment();
    assert!(root.is_some());
    assert_eq!(root.unwrap().database_id, 1);

    let replies = thread.replies();
    assert_eq!(replies.len(), 3);
    let reply_ids: Vec<i64> = replies.iter().map(|c| c.database_id).collect();
    assert!(reply_ids.contains(&2));
    assert!(reply_ids.contains(&3));
    assert!(reply_ids.contains(&4));
}

#[test]
fn test_reply_to_deserialize_ignores_extra_fields() {
    let json = r#"{"databaseId": 123}"#;
    let _reply_to: ReplyTo = serde_json::from_str(json).unwrap();
    assert_eq!(std::mem::size_of::<ReplyTo>(), 0);
}

#[test]
fn test_comment_with_reply_to_deserialize() {
    let json = r#"{
        "databaseId": 1,
        "author": {"login": "user"},
        "body": "test",
        "createdAt": "2024-01-01T00:00:00Z",
        "path": "file.rs",
        "line": 10,
        "originalLine": null,
        "diffHunk": null,
        "replyTo": {"databaseId": 999},
        "pullRequestReview": {"databaseId": 100}
    }"#;
    let comment: Comment = serde_json::from_str(json).unwrap();
    assert!(comment.reply_to.is_some());
    assert_eq!(comment.database_id, 1);
}
