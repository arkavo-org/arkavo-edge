#!/usr/bin/env node

/**
 * Verification script for ABAC policy JSON format
 *
 * This script demonstrates and validates the correct JSON format for AttributePolicy
 * serialization/deserialization in the WASM module.
 *
 * Key findings:
 * - Operator enum uses #[serde(rename_all = "camelCase")]
 * - LogicalOperator enum uses #[serde(rename_all = "UPPERCASE", tag = "type")]
 * - AttributePolicy is #[serde(untagged)] - no "type" field for simple conditions
 */

const { access_evaluate } = require('./pkg-node/opentdf_wasm.js');

console.log('🔍 ABAC Policy JSON Format Verification\n');

// Test cases with correct format
const testCases = [
    {
        name: 'Simple Condition (Equals)',
        policy: {
            attribute: { namespace: "gov.example", name: "clearance" },
            operator: "equals",  // camelCase!
            value: "TOP_SECRET"
        },
        userAttrs: { "gov.example:clearance": "TOP_SECRET" },
        expectedAccess: true
    },
    {
        name: 'Hierarchical Operator (MinimumOf)',
        policy: {
            attribute: { namespace: "gov.example", name: "clearance" },
            operator: "minimumOf",  // camelCase!
            value: "SECRET"
        },
        userAttrs: { "gov.example:clearance": "TOP_SECRET" },
        expectedAccess: true
    },
    {
        name: 'Array Operator (In)',
        policy: {
            attribute: { namespace: "gov.example", name: "department" },
            operator: "in",  // camelCase!
            value: ["ENGINEERING", "EXECUTIVE"]
        },
        userAttrs: { "gov.example:department": "ENGINEERING" },
        expectedAccess: true
    },
    {
        name: 'Logical AND with Multiple Conditions',
        policy: {
            type: "AND",  // UPPERCASE for logical operators
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
        },
        userAttrs: {
            "gov.example:clearance": "TOP_SECRET",
            "gov.example:department": "ENGINEERING"
        },
        expectedAccess: true
    },
    {
        name: 'Logical OR',
        policy: {
            type: "OR",  // UPPERCASE
            conditions: [
                {
                    attribute: { namespace: "gov.example", name: "role" },
                    operator: "equals",
                    value: "ADMIN"
                },
                {
                    attribute: { namespace: "gov.example", name: "role" },
                    operator: "equals",
                    value: "EXECUTIVE"
                }
            ]
        },
        userAttrs: { "gov.example:role": "EXECUTIVE" },
        expectedAccess: true
    },
    {
        name: 'Access Denied (Insufficient Clearance)',
        policy: {
            attribute: { namespace: "gov.example", name: "clearance" },
            operator: "equals",
            value: "TOP_SECRET"
        },
        userAttrs: { "gov.example:clearance": "SECRET" },
        expectedAccess: false
    }
];

let passed = 0;
let failed = 0;

testCases.forEach((test, index) => {
    console.log(`Test ${index + 1}: ${test.name}`);
    console.log(`Policy JSON: ${JSON.stringify(test.policy)}`);

    const result = access_evaluate(
        JSON.stringify(test.policy),
        JSON.stringify(test.userAttrs)
    );

    if (!result.success) {
        console.log(`❌ FAILED: ${result.error}`);
        failed++;
    } else {
        const access = result.data === 'true';
        if (access === test.expectedAccess) {
            console.log(`✅ PASSED: Access ${access ? 'granted' : 'denied'} as expected`);
            passed++;
        } else {
            console.log(`❌ FAILED: Expected access=${test.expectedAccess}, got ${access}`);
            failed++;
        }
    }
    console.log('');
});

console.log('━'.repeat(60));
console.log(`Results: ${passed}/${testCases.length} passed`);
if (failed > 0) {
    console.log(`⚠️  ${failed} test(s) failed`);
    process.exit(1);
} else {
    console.log('✅ All tests passed!');
}

// Print format reference
console.log('\n📚 Format Reference:\n');
console.log('Simple Condition:');
console.log(JSON.stringify({
    attribute: { namespace: "namespace", name: "attributeName" },
    operator: "camelCaseOperator",
    value: "value or array"
}, null, 2));

console.log('\nLogical Operator (AND/OR/NOT):');
console.log(JSON.stringify({
    type: "AND",  // or "OR", "NOT"
    conditions: [
        {
            attribute: { namespace: "namespace", name: "attributeName" },
            operator: "camelCaseOperator",
            value: "value"
        }
    ]
}, null, 2));

console.log('\nOperator Examples (all camelCase):');
console.log('  - equals, notEquals');
console.log('  - greaterThan, greaterThanOrEqual, lessThan, lessThanOrEqual');
console.log('  - contains, in, allOf, anyOf, notIn');
console.log('  - minimumOf, maximumOf (hierarchical)');
console.log('  - present, notPresent');
