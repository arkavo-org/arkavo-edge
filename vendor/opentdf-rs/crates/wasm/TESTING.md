# OpenTDF WASM Testing Guide

This guide explains how to test the OpenTDF WASM module in different environments.

## Browser Testing

### Quick Start

1. **Start the HTTP server** (required for WASM to work in browsers):
   ```bash
   cd crates/wasm
   python3 -m http.server 8000
   ```

2. **Open in browser**:
   Navigate to: http://localhost:8000/test-browser.html

3. **Run the tests**:
   - The page will automatically load the WASM module
   - Click the buttons to test different features
   - Check the results displayed on the page

### What Gets Tested

The browser test page (`test-browser.html`) includes:

1. ✅ **TDF Creation** - Create encrypted TDF archives
2. ✅ **TDF Reading** - Read and parse TDF manifests
3. ✅ **ABAC Policy Evaluation** - Test attribute-based access control
4. ✅ **Attribute Identifiers** - Parse attribute identifiers
5. ✅ **Policy Creation** - Create and validate policies

### Browser Requirements

- Modern browser with WebAssembly support:
  - Chrome/Edge 84+
  - Firefox 79+
  - Safari 15+

### Troubleshooting Browser Tests

**"WASM module failed to load"**
- Ensure you're using an HTTP server (not `file://` protocol)
- Check browser console for CORS errors
- Verify WASM file is accessible at `/pkg-web/opentdf_wasm_bg.wasm`

**"Cannot import from pkg-web"**
- Ensure you built with `wasm-pack build --target web`
- Check that pkg-web directory exists
- Verify the HTTP server is running in the correct directory

## Node.js Testing

### Quick Start

1. **Build for Node.js**:
   ```bash
   cd crates/wasm
   wasm-pack build --target nodejs --out-dir pkg-node --scope arkavo-org
   ```

2. **Run Node.js test**:
   ```bash
   node test-node.js
   ```

### Node.js Requirements

- Node.js 14+ with WebAssembly support
- No additional dependencies required

## Manual WASM Build

### Build for Web (Browser)

```bash
cd crates/wasm
wasm-pack build --target web --out-dir pkg-web --scope arkavo-org
```

Output:
- `pkg-web/opentdf_wasm.js` - ES module wrapper
- `pkg-web/opentdf_wasm_bg.wasm` - WASM binary (~550KB)
- `pkg-web/opentdf_wasm.d.ts` - TypeScript definitions
- `pkg-web/package.json` - NPM package metadata

### Build for Node.js

```bash
cd crates/wasm
wasm-pack build --target nodejs --out-dir pkg-node --scope arkavo-org
```

Output:
- `pkg-node/opentdf_wasm.js` - CommonJS wrapper
- `pkg-node/opentdf_wasm_bg.wasm` - WASM binary
- `pkg-node/opentdf_wasm.d.ts` - TypeScript definitions
- `pkg-node/package.json` - NPM package metadata

### Build Options

**Disable wasm-opt** (faster builds, larger binaries):
```bash
wasm-pack build --target web --out-dir pkg-web --scope arkavo-org -- --no-default-features
```

**Debug build** (with symbols):
```bash
wasm-pack build --dev --target web --out-dir pkg-web --scope arkavo-org
```

**Release build** (optimized):
```bash
wasm-pack build --release --target web --out-dir pkg-web --scope arkavo-org
```

## Performance Benchmarking

The browser test page includes performance metrics for all operations:

- **Load Time** - Time to initialize WASM module
- **TDF Creation** - Time to encrypt and create TDF
- **Manifest Reading** - Time to parse TDF manifest
- **Policy Evaluation** - Time to evaluate ABAC policies

Typical performance (measured on standard hardware):
- Module load: 50-150ms
- TDF creation: 5-15ms
- Manifest reading: 1-3ms
- Policy evaluation: <1ms

## Automated Testing

### Run WASM Tests

```bash
cd crates/wasm
wasm-pack test --node
```

This runs the Rust tests compiled to WASM.

### Run Clippy

```bash
cd crates/wasm
cargo clippy --target wasm32-unknown-unknown -- -D warnings
```

### Format Code

```bash
cd crates/wasm
cargo fmt
```

## Integration Testing

### Test with Real Data

1. Create a TDF:
   ```javascript
   const data = btoa('My sensitive data');
   const policy = { uuid: crypto.randomUUID(), body: { attributes: [], dissem: ['user@example.com'] } };
   const result = tdf_create(data, 'https://kas.example.com', JSON.stringify(policy));
   ```

2. Read it back:
   ```javascript
   const manifestResult = tdf_read(result.data);
   const manifest = JSON.parse(manifestResult.data);
   ```

3. Evaluate policy:
   ```javascript
   const userAttrs = { "gov.example:clearance": "SECRET" };
   const accessResult = access_evaluate(policyJson, JSON.stringify(userAttrs));
   ```

## CI/CD Testing

The GitHub Actions workflow automatically:
- Builds WASM for both web and Node.js targets
- Runs clippy and rustfmt checks
- Runs `wasm-pack test --node`
- Reports bundle sizes
- Uploads artifacts

See `.github/workflows/wasm-feature.yml` for details.

## Known Issues

1. **wasm-opt download failures**: Disabled by default in `Cargo.toml`
2. **Large bundle size**: ~550KB uncompressed (~200KB gzipped)
3. **No KAS support in WASM**: KAS features require async networking (not yet implemented)

## Getting Help

- Check browser console for errors
- Review the [WASM README](README.md)
- See [NPM Publishing Guide](NPM_PUBLISHING.md)
- Open an issue on GitHub

## Example Output

### Successful Browser Test

```
✅ WASM module loaded successfully!
Version: 0.3.0
WASM Size: 545.01 KB
Load Time: 87.32 ms

✅ TDF created successfully!
Size: 1234 bytes (base64)
Time: 12.45ms

✅ Manifest read successfully!
Time: 2.31ms

✅ Access GRANTED
Time: 0.87ms
Policy: clearance == TOP_SECRET
Result: true
```

### Performance Metrics

```
Create TDF: 12.45ms
Read TDF Manifest: 2.31ms
Simple Policy Evaluation: 0.87ms
Complex Policy Evaluation: 1.23ms
Create Attribute ID: 0.45ms
Create Policy: 0.62ms
```
