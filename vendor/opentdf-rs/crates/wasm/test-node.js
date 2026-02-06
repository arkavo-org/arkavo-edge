#!/usr/bin/env node

/**
 * Node.js test script for OpenTDF WASM
 *
 * Run with: node test-node.js
 */

const {
    version,
    tdf_create,
    tdf_read,
    access_evaluate,
    attribute_identifier_create,
    policy_create
} = require('./pkg-node/opentdf_wasm.js');

const crypto = require('crypto');

// ANSI color codes
const colors = {
    reset: '\x1b[0m',
    green: '\x1b[32m',
    red: '\x1b[31m',
    yellow: '\x1b[33m',
    blue: '\x1b[34m',
    cyan: '\x1b[36m'
};

function log(message, color = 'reset') {
    console.log(`${colors[color]}${message}${colors.reset}`);
}

function logTest(name) {
    console.log(`\n${colors.cyan}━━━ ${name} ━━━${colors.reset}`);
}

function logSuccess(message, time) {
    log(`✅ ${message} (${time.toFixed(2)}ms)`, 'green');
}

function logError(message) {
    log(`❌ ${message}`, 'red');
}

function logInfo(message) {
    log(`ℹ️  ${message}`, 'blue');
}

// Test counters
let passed = 0;
let failed = 0;

// Test wrapper
function runTest(name, testFn) {
    logTest(name);
    const start = performance.now();
    try {
        testFn();
        const duration = performance.now() - start;
        logSuccess(`PASSED`, duration);
        passed++;
    } catch (error) {
        logError(`FAILED: ${error.message}`);
        console.error(error);
        failed++;
    }
}

// Main test suite
async function runTests() {
    log('\n🔐 OpenTDF WASM Node.js Test Suite\n', 'cyan');

    // Display version
    const ver = version();
    logInfo(`OpenTDF WASM Version: ${ver}`);

    // Test 1: Create simple TDF
    let createdTdf = null;
    runTest('Test 1: Create Simple TDF', () => {
        const data = Buffer.from('Hello, OpenTDF WASM!').toString('base64');
        const kasUrl = 'https://kas.example.com';
        const policy = {
            uuid: crypto.randomUUID(),
            body: {
                attributes: [],
                dissem: ['user@example.com']
            }
        };

        const result = tdf_create(data, kasUrl, JSON.stringify(policy));

        if (!result.success) {
            throw new Error(result.error);
        }

        createdTdf = result.data;
        logInfo(`TDF size: ${result.data.length} bytes (base64)`);
    });

    // Test 2: Create TDF with ABAC policy
    runTest('Test 2: Create TDF with ABAC Policy', () => {
        const data = Buffer.from('Classified information').toString('base64');
        const kasUrl = 'https://kas.example.com';
        const policy = {
            uuid: crypto.randomUUID(),
            body: {
                attributes: [{
                    type: "And",
                    conditions: [{
                        type: "Condition",
                        attribute: {
                            namespace: "gov.example",
                            name: "clearance"
                        },
                        operator: "MinimumOf",
                        value: "SECRET"
                    }]
                }],
                dissem: ['user@example.com']
            }
        };

        const result = tdf_create(data, kasUrl, JSON.stringify(policy));

        if (!result.success) {
            throw new Error(result.error);
        }

        logInfo(`TDF with ABAC size: ${result.data.length} bytes`);
    });

    // Test 3: Read TDF manifest
    runTest('Test 3: Read TDF Manifest', () => {
        if (!createdTdf) {
            throw new Error('No TDF created to read');
        }

        const result = tdf_read(createdTdf);

        if (!result.success) {
            throw new Error(result.error);
        }

        const manifest = JSON.parse(result.data);
        logInfo(`Manifest has ${Object.keys(manifest).length} top-level keys`);
    });

    // Test 4: Evaluate simple ABAC policy
    runTest('Test 4: Evaluate Simple ABAC Policy', () => {
        const policy = {
            attribute: {
                namespace: "gov.example",
                name: "clearance"
            },
            operator: "equals",
            value: "TOP_SECRET"
        };

        const userAttrs = {
            "gov.example:clearance": "TOP_SECRET"
        };

        const result = access_evaluate(JSON.stringify(policy), JSON.stringify(userAttrs));

        if (!result.success) {
            throw new Error(result.error);
        }

        const granted = result.data === 'true';
        if (!granted) {
            throw new Error('Access should have been granted');
        }

        logInfo('Access granted as expected');
    });

    // Test 5: Evaluate complex ABAC policy
    runTest('Test 5: Evaluate Complex ABAC Policy', () => {
        const policy = {
            type: "AND",
            conditions: [
                {
                    attribute: { namespace: "gov.example", name: "clearance" },
                    operator: "minimumOf",
                    value: "SECRET"
                },
                {
                    attribute: { namespace: "gov.example", name: "department" },
                    operator: "equals",
                    value: "ENGINEERING"
                }
            ]
        };

        const userAttrs = {
            "gov.example:clearance": "TOP_SECRET",
            "gov.example:department": "ENGINEERING"
        };

        const result = access_evaluate(JSON.stringify(policy), JSON.stringify(userAttrs));

        if (!result.success) {
            throw new Error(result.error);
        }

        const granted = result.data === 'true';
        if (!granted) {
            throw new Error('Access should have been granted');
        }

        logInfo('Complex policy evaluation succeeded');
    });

    // Test 6: Deny access with insufficient attributes
    runTest('Test 6: Deny Access (Insufficient Attributes)', () => {
        const policy = {
            attribute: { namespace: "gov.example", name: "clearance" },
            operator: "equals",
            value: "TOP_SECRET"
        };

        const userAttrs = {
            "gov.example:clearance": "SECRET"  // Lower clearance
        };

        const result = access_evaluate(JSON.stringify(policy), JSON.stringify(userAttrs));

        if (!result.success) {
            throw new Error(result.error);
        }

        const granted = result.data === 'true';
        if (granted) {
            throw new Error('Access should have been denied');
        }

        logInfo('Access correctly denied');
    });

    // Test 7: Create attribute identifier
    runTest('Test 7: Create Attribute Identifier', () => {
        const attrId = 'gov.example:clearance';
        const result = attribute_identifier_create(attrId);

        if (!result.success) {
            throw new Error(result.error);
        }

        const parsed = JSON.parse(result.data);
        if (parsed.namespace !== 'gov.example' || parsed.name !== 'clearance') {
            throw new Error('Attribute identifier not parsed correctly');
        }

        logInfo(`Parsed: ${parsed.namespace}:${parsed.name}`);
    });

    // Test 8: Create and validate policy
    runTest('Test 8: Create and Validate Policy', () => {
        const policyJson = JSON.stringify({
            uuid: crypto.randomUUID(),
            body: {
                attributes: [],
                dissem: ['user@example.com']
            }
        });

        const result = policy_create(policyJson);

        if (!result.success) {
            throw new Error(result.error);
        }

        const policy = JSON.parse(result.data);
        if (!policy.uuid || !policy.body) {
            throw new Error('Policy structure invalid');
        }

        logInfo('Policy validated successfully');
    });

    // Test 9: Invalid attribute identifier
    runTest('Test 9: Invalid Attribute Identifier (Error Handling)', () => {
        const attrId = 'invalid-format';  // Missing colon
        const result = attribute_identifier_create(attrId);

        if (result.success) {
            throw new Error('Should have failed with invalid format');
        }

        logInfo('Error handled correctly');
    });

    // Test 10: Invalid policy JSON
    runTest('Test 10: Invalid Policy JSON (Error Handling)', () => {
        const invalidJson = '{invalid json}';
        const result = policy_create(invalidJson);

        if (result.success) {
            throw new Error('Should have failed with invalid JSON');
        }

        logInfo('Error handled correctly');
    });

    // Print summary
    console.log(`\n${'='.repeat(50)}`);
    log(`\n📊 Test Results:\n`, 'cyan');
    logSuccess(`Passed: ${passed}`, 0);
    if (failed > 0) {
        logError(`Failed: ${failed}`);
    }

    const total = passed + failed;
    const percentage = ((passed / total) * 100).toFixed(1);
    log(`\nTotal: ${passed}/${total} (${percentage}%)`, percentage == 100 ? 'green' : 'yellow');

    console.log(`${'='.repeat(50)}\n`);

    // Exit with appropriate code
    process.exit(failed > 0 ? 1 : 0);
}

// Run the tests
runTests().catch(error => {
    logError(`Fatal error: ${error.message}`);
    console.error(error);
    process.exit(1);
});
