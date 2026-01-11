use serial_test::serial;

use crate::testing::TestRepo;
use crate::wm::git::{branch_to_worktree_name, get_repo_root};

#[test]
#[serial]
fn get_repo_root_from_main_returns_main_path() {
    let repo = TestRepo::new();

    repo.run_in_dir(|| {
        let root = get_repo_root().unwrap();
        assert_eq!(root, repo.path().to_string_lossy());
    });
}

#[test]
#[serial]
fn get_repo_root_from_worktree_returns_main_path() {
    let repo = TestRepo::new();

    // Create a worktree
    repo.create_worktree("test-branch");

    // Run from inside the worktree
    repo.run_in_worktree("test-branch", || {
        let root = get_repo_root().unwrap();
        // Should return the main repo path, not the worktree path
        assert_eq!(root, repo.path().to_string_lossy());
    });
}

#[test]
#[serial]
fn worktrees_dir_created_in_main_when_run_from_worktree() {
    let repo = TestRepo::new();

    // Create first worktree manually
    repo.create_worktree("first");

    // From inside first worktree, create another worktree using our logic
    repo.run_in_worktree("first", || {
        let root = get_repo_root().unwrap();
        let worktrees_dir = format!("{root}/.worktrees");

        // .worktrees should be in the main repo, not in the worktree
        assert!(
            worktrees_dir.starts_with(&repo.path().to_string_lossy().to_string()),
            "worktrees_dir ({worktrees_dir}) should be under main repo ({})",
            repo.path().display()
        );
    });
}

#[test]
fn branch_to_worktree_name_removes_prefix_and_slashes() {
    assert_eq!(branch_to_worktree_name("feature"), "feature");
    assert_eq!(branch_to_worktree_name("fohte/feature"), "feature");
    assert_eq!(branch_to_worktree_name("feature/sub"), "feature-sub");
    assert_eq!(branch_to_worktree_name("fohte/feature/sub"), "feature-sub");
}
