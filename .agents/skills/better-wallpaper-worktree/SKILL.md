---
name: better-wallpaper-worktree
description: Run a task in an isolated git worktree when the user prefixes a request with `wt:`. Use only for the wt: trigger; creates a worktree from dev, completes and commits the task there, rebases onto dev, then removes the worktree and its branch.
---

# Worktree Task Isolation

A message beginning with `wt:` requests that the following task run in a
dedicated git worktree instead of the current checkout. The worktree is
temporary: it starts from `dev`, the finished work is rebased onto `dev`,
and the worktree and its branch are deleted afterwards.

Follow repository `AGENTS.md`. That file prohibits subagents for this project,
so the `wt:` workflow isolates one task in a separate working tree; it does not
parallelize work across agents.

## Invocation

Take the task from the text after the `wt:` prefix. The remaining message is
context and constraints for that task. Do not treat the prefix as a branch name.

Derive a short slug from the task, for example `wt: add plasma hotplug delay`
becomes `plasma-hotplug-delay`. Use `.worktrees/<slug>` for the worktree and
`wt/<slug>` for the branch. A slug must not start with `.` and must not end with
`.lock`; git rejects both as branch names, so pick a valid slug before creating
anything.

`.worktrees/` is ignored by git. If it is missing from `.gitignore`, add it in
the primary checkout first, because a nested worktree would otherwise appear as
an untracked directory there.

## Workflow

Run the task's own validation inside the worktree, not in the primary checkout.
Load the skills that the task's category requires, as `AGENTS.md` directs; this
skill replaces `better-wallpaper-*` category skills only for their setup steps,
never for their contracts or verification requirements.

1. **Create.** Confirm `dev` exists and the slug is unused:
   `git worktree add -b wt/<slug> .worktrees/<slug> dev`. Worktree checkouts
   contain only tracked files, so artifacts ignored by git in the primary
   checkout (build output, installed dependencies, local configuration) are
   absent and must be regenerated or reinstalled inside the worktree.
2. **Implement.** Work only inside `.worktrees/<slug>`. Commit finished work with
   the `AGENTS.md` prefixes and split independent changes into separate commits.
   Leave nothing uncommitted, because the cleanup steps discard the worktree.
3. **Rebase.** Keep the worktree intact until this and the next step both
   succeed, so a failure never strands the work.
   `git -C .worktrees/<slug> rebase dev`. On a conflict, stop, report the
   conflicting files, and leave the worktree in place for manual resolution.
4. **Advance dev.** From the primary checkout, fast-forward `dev` to the rebased
   branch: `git merge --ff-only wt/<slug>`. Then confirm `dev` resolves to the
   worktree's HEAD. Do not use `git push . HEAD:dev`, which git rejects while
   `dev` is checked out in the primary worktree. A fast-forward merge updates
   the index, so it stays consistent with `dev` and preserves unrelated
   uncommitted work; moving the `dev` ref directly with `git update-ref` does
   not, and leaves that work looking staged or deleted.
5. **Delete.** `git worktree remove .worktrees/<slug>`, then delete the branch
   with `git branch -D wt/<slug>`, then `git worktree prune`. Force-deleting is
   safe because step 4 already put the branch's commits on `dev`, and it keeps
   cleanup working when `dev` is not the currently checked-out branch.

Do not force or discard work to make a step pass. If a commit cannot be created,
the rebase conflicts, or `dev` has diverged, report the state and stop with the
worktree still present.

## Cleanup Notes

`git worktree remove` refuses a worktree containing modified or untracked files,
including build output that is ignored by git. After step 4 has succeeded, delete
such a worktree with `--force` and report it; the committed work is already on
`dev` and the branch tip is preserved until step 5 completes.

Removing the worktree does not update a separate `dev` checkout. Report that
whenever `dev` is checked out elsewhere, so the user can refresh it.

## Verification

The workflow is complete only when the worktree list no longer contains the task
worktree, the `wt/<slug>` branch is gone, and `dev` contains the task's commits.
Report every validation command actually run, following `AGENTS.md`, and state
accurately which checks could not run.
