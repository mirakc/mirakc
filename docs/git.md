# Git Repository Management

## The `release` branch

The `release` branch is a special branch.  It's replaced with a branch for the
latest release every major or minor release.

The following steps are replacing the `release` branch with a newly created
release branch

```shell
./script/release minor
git branch -m release release-old
git branch release
git push -u origin main
git push -fu origin release
git push --tags
```

> TODO: Automate the workflow using the GitHub Actions

## Release tags

Release tags will be created only on the `release` branch every Friday if there
are any commits from the last Friday.  The patch number will be updated
automatically in the [GitHub weekly workflow].

[GitHub weekly workflow]: https://github.com/mirakc/mirakc/actions/workflows/weekly.yml
