# Publishing and Using @arkavo-org/opentdf-wasm

This guide explains how to publish and use the `@arkavo-org/opentdf-wasm` package from GitHub Packages.

## Automated Publishing

The WASM package is automatically published to GitHub Packages through GitHub Actions workflows:

### When Packages Are Published

1. **Main Branch Pushes**: When changes are pushed to the `main` branch
2. **Tagged Releases**: When a git tag (e.g., `v0.3.0`) is created

### What Gets Published

- **@arkavo-org/opentdf-wasm** - Two separate builds:
  - Web target (ES modules for browsers)
  - Node.js target (CommonJS for Node.js)

Both builds are published as the same package name but wasm-pack handles the different targets.

## Installing from GitHub Packages

### Prerequisites

You need a GitHub personal access token (PAT) with `read:packages` permission to install packages from GitHub Packages.

### Step 1: Create a Personal Access Token

1. Go to GitHub Settings → Developer settings → Personal access tokens → Tokens (classic)
2. Click "Generate new token (classic)"
3. Give it a name like "npm-packages-read"
4. Select the `read:packages` scope
5. Click "Generate token"
6. Copy the token (you won't see it again!)

### Step 2: Configure npm

Create or update your `~/.npmrc` file (or project-level `.npmrc`):

```
@arkavo-org:registry=https://npm.pkg.github.com
//npm.pkg.github.com/:_authToken=YOUR_GITHUB_TOKEN_HERE
```

**Important**: Never commit `.npmrc` with your token to version control!

Alternatively, you can use environment variables:

```bash
# Set the token as an environment variable
export NPM_TOKEN=your_github_token_here

# Use in .npmrc
echo "//npm.pkg.github.com/:_authToken=\${NPM_TOKEN}" >> .npmrc
```

### Step 3: Install the Package

```bash
npm install @arkavo-org/opentdf-wasm
```

Or with Yarn:

```bash
yarn add @arkavo-org/opentdf-wasm
```

## Using in CI/CD

### GitHub Actions

GitHub Actions automatically has access to `GITHUB_TOKEN` with package permissions:

```yaml
- name: Setup Node.js
  uses: actions/setup-node@v4
  with:
    node-version: '20'
    registry-url: 'https://npm.pkg.github.com'
    scope: '@arkavo-org'

- name: Install dependencies
  run: npm install
  env:
    NODE_AUTH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

### Other CI Systems

For other CI systems (GitLab, Jenkins, etc.), create a GitHub PAT and add it as a secret:

```yaml
# Example for GitLab CI
install:
  script:
    - echo "@arkavo-org:registry=https://npm.pkg.github.com" >> .npmrc
    - echo "//npm.pkg.github.com/:_authToken=${GITHUB_TOKEN}" >> .npmrc
    - npm install
```

## Publishing Manually (Maintainers Only)

If you need to publish manually:

### Prerequisites

1. You must have write access to the repository
2. You need a GitHub PAT with `write:packages` and `read:packages` permissions

### Steps

```bash
# 1. Build the WASM packages
cd crates/wasm
wasm-pack build --release --target web --out-dir pkg-web --scope arkavo-org
wasm-pack build --release --target nodejs --out-dir pkg-node --scope arkavo-org

# 2. Configure npm authentication
export NODE_AUTH_TOKEN=your_github_token_here

# 3. Publish web target
cd pkg-web
npm publish --registry=https://npm.pkg.github.com

# 4. Publish Node.js target
cd ../pkg-node
npm publish --registry=https://npm.pkg.github.com
```

## Version Management

### Updating the Version

1. Update version in `crates/wasm/Cargo.toml`
2. Update version in `crates/wasm/package.json`
3. Commit the changes
4. Create a git tag:

```bash
git tag v0.3.1
git push origin v0.3.1
```

The GitHub Actions workflow will automatically build and publish the new version.

### Version Strategy

- Follow [Semantic Versioning](https://semver.org/)
- Keep versions in sync between Cargo.toml and package.json
- Use git tags for releases

## Troubleshooting

### "404 Not Found" When Installing

- Ensure you have read access to the `arkavo-org/opentdf-rs` repository
- Verify your `.npmrc` is configured correctly
- Check that your GitHub token is valid and has `read:packages` permission

### "401 Unauthorized" When Publishing

- Ensure you have write access to the repository
- Verify your token has `write:packages` permission
- Check that you're using the correct registry URL

### Package Not Found After Publishing

- GitHub Packages may take a few minutes to index new packages
- Verify the package was published: https://github.com/orgs/arkavo-org/packages
- Check the workflow logs for errors

### Installing Specific Versions

```bash
# Install specific version
npm install @arkavo-org/opentdf-wasm@0.3.0

# Install latest
npm install @arkavo-org/opentdf-wasm@latest
```

## Public vs Private Packages

GitHub Packages for public repositories are free and can be downloaded by anyone with a GitHub account. However, users still need a GitHub token for authentication.

To make installation easier for public use, consider also publishing to:
- [npmjs.com](https://www.npmjs.com/) - Public npm registry (no auth required)
- GitHub Releases - Download tarball directly

## Alternative: Installing from GitHub Releases

Users can also download pre-built packages from GitHub Releases without needing npm:

```bash
# Download the combined package
wget https://github.com/arkavo-org/opentdf-rs/releases/download/v0.3.0/opentdf-wasm-combined.tar.gz

# Extract
tar -xzf opentdf-wasm-combined.tar.gz

# Use in your project
# The package contains web/ and node/ directories
```

## Resources

- [GitHub Packages Documentation](https://docs.github.com/en/packages)
- [Working with the npm registry](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-npm-registry)
- [wasm-pack Documentation](https://rustwasm.github.io/wasm-pack/)
