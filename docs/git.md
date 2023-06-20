# Git Repository Management

## Release branches

Branches whose names start with `release-` are release branches.  For example,
`release-2.2` is the release branch for the version `2.2`.

The `release` branch is a special one.  Actually it's not a real branch,  it's
just a symbolic reference to the `HEAD` of the latest branch.

The following steps are updating the `release` branch with a newly created
release branch:

```shell
CURRENT=$(cargo metadata --no-deps --format-version=1 | \
            jq -r '.packages[] | select(.name == "mirakc") | .version')
VERSION=$(npx semver $CURRENT -i minor)
MAJOR=$(echo $VERSION | cut -d '.' -f 1)
MINOR=$(echo $VERSION | cut -d '.' -f 2)
BRANCH=release-$MAJOR.$MINOR

# Create a new release branch and update the `release` branch.
git checkout -b $BRANCH
git symbolic-ref refs/heads/release refs/heads/$BRANCH

# Update the version numbers (cargo-edit is needed).
cargo set-version $VERSION \
  --exclude=actlet --exclude=actlet-derive --exclude=chrono-jst

git add .
git commit -m "release: bump version to $VERSION"
git tag -a $VERSION -m "release: $VERSION"

git push -u origin $BRANCH
git push -u -f origin release
git push --tags
```

> TODO: Automate the workflow using the GitHub Actions

## Release tags

Release tags will be created only on the `release` branch every Friday if there
are any commits from the last Friday.  The patch number will be updated
automatically in the [GitHub weekly workflow].

[GitHub weekly workflow]: https://github.com/mirakc/mirakc/actions/workflows/weekly.yml
