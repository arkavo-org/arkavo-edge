#!/usr/bin/env node

/**
 * OpenTDF WASM Server
 *
 * A simple HTTP server demonstrating server-side WASM usage
 *
 * Run with: node server.js
 * Test with: curl http://localhost:3000/api/version
 */

const http = require('http');
const crypto = require('crypto');

// Import WASM module
const {
    version,
    tdf_create,
    tdf_read,
    access_evaluate,
    attribute_identifier_create,
    policy_create
} = require('./pkg-node/opentdf_wasm.js');

const PORT = 3000;

// ANSI colors for logging
const colors = {
    reset: '\x1b[0m',
    green: '\x1b[32m',
    blue: '\x1b[36m',
    yellow: '\x1b[33m',
    red: '\x1b[31m'
};

function log(message, color = 'reset') {
    const timestamp = new Date().toISOString();
    console.log(`${colors[color]}[${timestamp}] ${message}${colors.reset}`);
}

// Helper to parse request body
function parseBody(req) {
    return new Promise((resolve, reject) => {
        let body = '';
        req.on('data', chunk => {
            body += chunk.toString();
        });
        req.on('end', () => {
            try {
                resolve(body ? JSON.parse(body) : {});
            } catch (error) {
                reject(new Error('Invalid JSON'));
            }
        });
        req.on('error', reject);
    });
}

// Helper to send JSON response
function sendJSON(res, statusCode, data) {
    res.writeHead(statusCode, {
        'Content-Type': 'application/json',
        'Access-Control-Allow-Origin': '*',
        'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
        'Access-Control-Allow-Headers': 'Content-Type'
    });
    res.end(JSON.stringify(data, null, 2));
}

// Helper to send error response
function sendError(res, statusCode, message) {
    sendJSON(res, statusCode, {
        error: message,
        timestamp: new Date().toISOString()
    });
}

// Route handlers
const routes = {
    // GET /api/version - Get WASM module version
    'GET /api/version': (req, res) => {
        const ver = version();
        const wasmInfo = {
            version: ver,
            platform: 'wasm32-unknown-unknown',
            runtime: 'Node.js',
            nodeVersion: process.version,
            timestamp: new Date().toISOString()
        };
        sendJSON(res, 200, wasmInfo);
    },

    // GET /api/health - Health check
    'GET /api/health': (req, res) => {
        sendJSON(res, 200, {
            status: 'healthy',
            wasm: 'loaded',
            version: version(),
            uptime: process.uptime(),
            memory: process.memoryUsage()
        });
    },

    // POST /api/attribute/parse - Parse attribute identifier
    'POST /api/attribute/parse': async (req, res) => {
        try {
            const body = await parseBody(req);
            const { identifier } = body;

            if (!identifier) {
                return sendError(res, 400, 'Missing required field: identifier');
            }

            const result = attribute_identifier_create(identifier);

            if (result.success) {
                const parsed = JSON.parse(result.data);
                sendJSON(res, 200, {
                    success: true,
                    input: identifier,
                    parsed: parsed
                });
            } else {
                sendError(res, 400, result.error);
            }
        } catch (error) {
            sendError(res, 500, error.message);
        }
    },

    // POST /api/policy/create - Create and validate policy
    'POST /api/policy/create': async (req, res) => {
        try {
            const body = await parseBody(req);

            // Auto-generate UUID if not provided
            if (!body.uuid) {
                body.uuid = crypto.randomUUID();
            }

            // Ensure body structure
            if (!body.body) {
                body.body = {
                    attributes: body.attributes || [],
                    dissem: body.dissem || []
                };
            }

            const result = policy_create(JSON.stringify(body));

            if (result.success) {
                const policy = JSON.parse(result.data);
                sendJSON(res, 201, {
                    success: true,
                    policy: policy
                });
            } else {
                sendError(res, 400, result.error);
            }
        } catch (error) {
            sendError(res, 500, error.message);
        }
    },

    // POST /api/policy/validate - Validate existing policy
    'POST /api/policy/validate': async (req, res) => {
        try {
            const body = await parseBody(req);
            const result = policy_create(JSON.stringify(body));

            if (result.success) {
                sendJSON(res, 200, {
                    valid: true,
                    policy: JSON.parse(result.data)
                });
            } else {
                sendJSON(res, 200, {
                    valid: false,
                    error: result.error
                });
            }
        } catch (error) {
            sendError(res, 500, error.message);
        }
    },

    // POST /api/access/evaluate - Evaluate ABAC policy
    'POST /api/access/evaluate': async (req, res) => {
        try {
            const body = await parseBody(req);
            const { policy, attributes } = body;

            if (!policy || !attributes) {
                return sendError(res, 400, 'Missing required fields: policy, attributes');
            }

            const result = access_evaluate(
                JSON.stringify(policy),
                JSON.stringify(attributes)
            );

            if (result.success) {
                const granted = result.data === 'true';
                sendJSON(res, 200, {
                    success: true,
                    decision: granted ? 'ALLOW' : 'DENY',
                    granted: granted,
                    policy: policy,
                    attributes: attributes,
                    timestamp: new Date().toISOString()
                });
            } else {
                sendError(res, 400, result.error);
            }
        } catch (error) {
            sendError(res, 500, error.message);
        }
    },

    // GET /api/examples - Get example requests
    'GET /api/examples': (req, res) => {
        const examples = {
            version: {
                method: 'GET',
                url: '/api/version',
                description: 'Get WASM module version'
            },
            parseAttribute: {
                method: 'POST',
                url: '/api/attribute/parse',
                description: 'Parse attribute identifier',
                body: {
                    identifier: 'gov.example:clearance'
                }
            },
            createPolicy: {
                method: 'POST',
                url: '/api/policy/create',
                description: 'Create new policy',
                body: {
                    dissem: ['user@example.com'],
                    attributes: []
                }
            },
            evaluateAccess: {
                method: 'POST',
                url: '/api/access/evaluate',
                description: 'Evaluate ABAC policy',
                body: {
                    policy: {
                        type: 'Condition',
                        attribute: {
                            namespace: 'gov.example',
                            name: 'clearance'
                        },
                        operator: 'Equals',
                        value: 'SECRET'
                    },
                    attributes: {
                        'gov.example:clearance': 'SECRET'
                    }
                }
            }
        };
        sendJSON(res, 200, examples);
    },

    // GET / - API documentation
    'GET /': (req, res) => {
        const docs = {
            name: 'OpenTDF WASM Server',
            version: version(),
            description: 'REST API demonstrating server-side WASM usage',
            endpoints: [
                'GET  /api/version          - Get version info',
                'GET  /api/health           - Health check',
                'GET  /api/examples         - Get example requests',
                'POST /api/attribute/parse  - Parse attribute identifier',
                'POST /api/policy/create    - Create policy',
                'POST /api/policy/validate  - Validate policy',
                'POST /api/access/evaluate  - Evaluate ABAC policy'
            ],
            documentation: 'See /api/examples for request examples'
        };
        sendJSON(res, 200, docs);
    }
};

// Request handler
const server = http.createServer(async (req, res) => {
    const routeKey = `${req.method} ${req.url}`;

    log(`${req.method} ${req.url}`, 'blue');

    // Handle CORS preflight
    if (req.method === 'OPTIONS') {
        res.writeHead(204, {
            'Access-Control-Allow-Origin': '*',
            'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
            'Access-Control-Allow-Headers': 'Content-Type'
        });
        res.end();
        return;
    }

    const handler = routes[routeKey];

    if (handler) {
        try {
            await handler(req, res);
            log(`✓ ${routeKey} - 200`, 'green');
        } catch (error) {
            log(`✗ ${routeKey} - Error: ${error.message}`, 'red');
            sendError(res, 500, error.message);
        }
    } else {
        log(`✗ ${routeKey} - 404 Not Found`, 'yellow');
        sendError(res, 404, 'Route not found');
    }
});

// Start server
server.listen(PORT, () => {
    console.log('\n' + '='.repeat(60));
    log('🚀 OpenTDF WASM Server Started', 'green');
    console.log('='.repeat(60));
    log(`📍 Address: http://localhost:${PORT}`, 'blue');
    log(`📦 WASM Version: ${version()}`, 'blue');
    log(`🔧 Runtime: Node.js ${process.version}`, 'blue');
    log(`💾 Memory: ${(process.memoryUsage().heapUsed / 1024 / 1024).toFixed(2)} MB`, 'blue');
    console.log('='.repeat(60));
    console.log('\n📚 Available Endpoints:');
    console.log('  GET  http://localhost:3000/');
    console.log('  GET  http://localhost:3000/api/version');
    console.log('  GET  http://localhost:3000/api/health');
    console.log('  GET  http://localhost:3000/api/examples');
    console.log('  POST http://localhost:3000/api/attribute/parse');
    console.log('  POST http://localhost:3000/api/policy/create');
    console.log('  POST http://localhost:3000/api/access/evaluate');
    console.log('\n💡 Try: curl http://localhost:3000/api/version');
    console.log('='.repeat(60) + '\n');
    log('Server ready to accept connections', 'green');
});

// Graceful shutdown
process.on('SIGINT', () => {
    log('\n🛑 Shutting down server...', 'yellow');
    server.close(() => {
        log('Server closed', 'green');
        process.exit(0);
    });
});

process.on('SIGTERM', () => {
    log('\n🛑 Shutting down server...', 'yellow');
    server.close(() => {
        log('Server closed', 'green');
        process.exit(0);
    });
});
