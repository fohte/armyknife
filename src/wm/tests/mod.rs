use crate::testing::TestRepo;
use crate::wm::git::{branch_to_worktree_name, get_repo_root_in};

#[test]
fn get_repo_root_from_main_returns_main_path() {
    let repo = TestRepo::new();

    let root = get_repo_root_in(&repo.path()).unwrap();
    assert_eq!(root, repo.path().to_string_lossy().as_ref());
}

#[test]
fn get_repo_root_from_worktree_returns_main_path() {
    let repo = TestRepo::new();
    repo.create_worktree("test-branch");

    // Run git from inside the worktree, should still return main repo path
    let root = get_repo_root_in(&repo.worktree_path("test-branch")).unwrap();
    assert_eq!(root, repo.path().to_string_lossy().as_ref());
}

#[test]
fn worktrees_dir_created_in_main_when_run_from_worktree() {
    let repo = TestRepo::new();
    repo.create_worktree("first");

    // From inside first worktree, get_repo_root_in should return main repo
    let root = get_repo_root_in(&repo.worktree_path("first")).unwrap();
    let worktrees_dir = format!("{root}/.worktrees");

    // .worktrees should be in the main repo, not in the worktree
    assert!(
        worktrees_dir.starts_with(&repo.path().to_string_lossy().to_string()),
        "worktrees_dir ({worktrees_dir}) should be under main repo ({})",
        repo.path().display()
    );
}

#[test]
fn branch_to_worktree_name_removes_prefix_and_slashes() {
    assert_eq!(branch_to_worktree_name("feature"), "feature");
    assert_eq!(branch_to_worktree_name("fohte/feature"), "feature");
    assert_eq!(branch_to_worktree_name("feature/sub"), "feature-sub");
    assert_eq!(branch_to_worktree_name("fohte/feature/sub"), "feature-sub");
}
