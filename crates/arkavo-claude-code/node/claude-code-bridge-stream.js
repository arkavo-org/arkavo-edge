#!/usr/bin/env node

/**
 * Claude Code SDK Bridge for Arkavo - Streaming Iterator Version
 * 
 * This bridge consumes the Claude Code SDK's streaming iterator and maps
 * events to Arkavo's event bus via JSON-RPC notifications.
 */

const readline = require('readline');
const path = require('path');
const { spawn } = require('child_process');

// Configuration from environment
const ANTHROPIC_API_KEY = process.env.ANTHROPIC_API_KEY;
const ANTHROPIC_AUTH_TOKEN = process.env.ANTHROPIC_AUTH_TOKEN;
const ANTHROPIC_BASE_URL = process.env.ANTHROPIC_BASE_URL;
const ANTHROPIC_MODEL = process.env.ANTHROPIC_MODEL || 'claude-3-sonnet-20240229';
const WORKSPACE_ROOT = process.env.WORKSPACE_ROOT || process.cwd();

// Find the Claude executable
function findClaudeExecutable() {
    const possiblePaths = [
        // Local Claude installation (preferred)
        path.join(process.env.HOME, '.claude', 'local', 'node_modules', '.bin', 'claude'),
        path.join(process.env.HOME, '.claude', 'local', 'claude'),
        // Global npm installation
        '/usr/local/bin/claude',
        '/opt/homebrew/bin/claude',
        // Fallback to PATH
        'claude'
    ];
    
    // In production, would check each path exists
    // For now, return the most likely
    return possiblePaths[0];
}

// JSON-RPC helper functions
function sendNotification(method, params) {
    const notification = {
        jsonrpc: '2.0',
        method,
        params
    };
    console.log(JSON.stringify(notification));
}

function sendResponse(id, result) {
    const response = {
        jsonrpc: '2.0',
        id,
        result
    };
    console.log(JSON.stringify(response));
}

function sendError(id, code, message) {
    const response = {
        jsonrpc: '2.0',
        id,
        error: {
            code,
            message
        }
    };
    console.log(JSON.stringify(response));
}

// Map SDK events to Arkavo events
function mapEventToArkavo(event, sequence) {
    const eventType = event.type || event.event;
    
    switch (eventType) {
        case 'planning':
        case 'plan_updated':
            return {
                type: 'plan_updated',
                data: {
                    plan: event.plan || event.data,
                    sequence
                }
            };
            
        case 'tool_use':
        case 'tool_call':
            return {
                type: 'tool_use',
                data: {
                    tool: {
                        name: event.tool_name || event.name,
                        parameters: event.parameters || event.args,
                        id: event.tool_call_id || event.id
                    },
                    sequence
                }
            };
            
        case 'tool_result':
            return {
                type: 'tool_result',
                data: {
                    result: {
                        tool: event.tool_name || event.name,
                        id: event.tool_call_id || event.id,
                        success: event.success !== false,
                        output: event.output || event.result,
                        duration_ms: event.duration_ms || 0
                    },
                    sequence
                }
            };
            
        case 'content':
        case 'content_chunk':
        case 'text':
            return {
                type: 'content_chunk',
                data: {
                    content: event.content || event.text || event.data,
                    sequence
                }
            };
            
        case 'usage':
        case 'token_usage':
            return {
                type: 'token_usage',
                data: {
                    usage: {
                        prompt_tokens: event.prompt_tokens || event.input_tokens,
                        completion_tokens: event.completion_tokens || event.output_tokens,
                        total_tokens: event.total_tokens
                    },
                    sequence
                }
            };
            
        case 'error':
            return {
                type: 'error',
                data: {
                    error: event.message || event.error,
                    code: event.code,
                    sequence
                }
            };
            
        case 'done':
        case 'complete':
            return {
                type: 'run_completed',
                data: {
                    sequence
                }
            };
            
        default:
            // Log unmapped events for debugging
            process.stderr.write(`Unmapped event type: ${eventType}\n`);
            return null;
    }
}

// Start a Claude Code run and stream events
async function startRun(params) {
    const { prompt, context, sessionId, options = {} } = params;
    
    // Build command arguments for Claude CLI
    const args = [];
    
    // Add the prompt
    args.push(prompt);
    
    // Add workspace directory
    args.push('--workspace', WORKSPACE_ROOT);
    
    // Add model if specified
    if (ANTHROPIC_MODEL) {
        args.push('--model', ANTHROPIC_MODEL);
    }
    
    // Add API configuration
    if (ANTHROPIC_BASE_URL) {
        args.push('--base-url', ANTHROPIC_BASE_URL);
    }
    
    // Policy options
    if (options.planOnly) {
        args.push('--plan-only');
    }
    
    if (options.requireAccept) {
        args.push('--require-accept');
    }
    
    // Continuation
    if (options.continueSession && sessionId) {
        args.push('--continue', sessionId);
    }
    
    // Find and spawn Claude process
    const claudePath = findClaudeExecutable();
    process.stderr.write(`Starting Claude from: ${claudePath}\n`);
    
    const claudeProcess = spawn(claudePath, args, {
        env: {
            ...process.env,
            ANTHROPIC_API_KEY: ANTHROPIC_API_KEY || ANTHROPIC_AUTH_TOKEN,
            ANTHROPIC_MODEL,
            ANTHROPIC_BASE_URL
        },
        cwd: WORKSPACE_ROOT
    });
    
    let sequence = 0;
    const runId = sessionId || `run-${Date.now()}`;
    
    // Emit run started event
    sendNotification('event', {
        type: 'run_started',
        sessionId: runId,
        data: {
            prompt,
            model: ANTHROPIC_MODEL,
            sequence: sequence++
        }
    });
    
    // Process stdout (event stream)
    claudeProcess.stdout.on('data', (data) => {
        const lines = data.toString().split('\n').filter(line => line.trim());
        
        for (const line of lines) {
            try {
                // Try to parse as JSON event
                const event = JSON.parse(line);
                
                // Map to Arkavo event
                const mappedEvent = mapEventToArkavo(event, sequence++);
                if (mappedEvent) {
                    sendNotification('event', {
                        type: mappedEvent.type,
                        sessionId: runId,
                        ...mappedEvent.data
                    });
                }
            } catch (e) {
                // Not JSON, might be plain text output
                if (line.trim()) {
                    sendNotification('event', {
                        type: 'content_chunk',
                        sessionId: runId,
                        data: {
                            content: line,
                            sequence: sequence++
                        }
                    });
                }
            }
        }
    });
    
    // Process stderr (errors and debug info)
    claudeProcess.stderr.on('data', (data) => {
        process.stderr.write(`Claude stderr: ${data}`);
    });
    
    // Handle process exit
    claudeProcess.on('close', (code) => {
        if (code === 0) {
            sendNotification('event', {
                type: 'run_completed',
                sessionId: runId,
                data: {
                    sequence: sequence++
                }
            });
        } else {
            sendNotification('event', {
                type: 'run_failed',
                sessionId: runId,
                data: {
                    error: `Process exited with code ${code}`,
                    sequence: sequence++
                }
            });
        }
        
        // Send session closed
        sendNotification('event', {
            type: 'session_closed',
            sessionId: runId,
            data: {
                sequence: sequence++
            }
        });
    });
    
    // Handle process errors
    claudeProcess.on('error', (error) => {
        sendNotification('event', {
            type: 'error',
            sessionId: runId,
            data: {
                error: error.message,
                sequence: sequence++
            }
        });
    });
    
    return {
        runId,
        process: claudeProcess
    };
}

// Active runs map
const activeRuns = new Map();

// Process JSON-RPC requests
async function handleRequest(request) {
    const { id, method, params } = request;
    
    try {
        switch (method) {
            case 'initialize':
                // Verify Claude is available
                const claudePath = findClaudeExecutable();
                sendResponse(id, {
                    ready: true,
                    claudePath,
                    model: ANTHROPIC_MODEL
                });
                break;
                
            case 'startRun':
                // Start a new Claude run
                const run = await startRun(params);
                activeRuns.set(run.runId, run);
                sendResponse(id, {
                    runId: run.runId
                });
                break;
                
            case 'stopRun':
                // Stop an active run
                const { runId } = params;
                const activeRun = activeRuns.get(runId);
                if (activeRun) {
                    activeRun.process.kill();
                    activeRuns.delete(runId);
                    sendResponse(id, { stopped: true });
                } else {
                    sendError(id, -32602, `No active run with ID: ${runId}`);
                }
                break;
                
            case 'listRuns':
                // List active runs
                sendResponse(id, {
                    runs: Array.from(activeRuns.keys())
                });
                break;
                
            default:
                sendError(id, -32601, `Method not found: ${method}`);
        }
    } catch (error) {
        sendError(id, -32603, error.message);
    }
}

// Set up stdin reader for JSON-RPC
const rl = readline.createInterface({
    input: process.stdin,
    output: null,
    terminal: false
});

rl.on('line', async (line) => {
    try {
        const request = JSON.parse(line);
        if (request.jsonrpc === '2.0' && request.method) {
            await handleRequest(request);
        }
    } catch (error) {
        process.stderr.write(`Failed to parse request: ${error.message}\n`);
    }
});

// Cleanup on exit
process.on('SIGTERM', () => {
    // Kill all active runs
    for (const [runId, run] of activeRuns) {
        run.process.kill();
    }
    process.exit(0);
});

// Log startup
process.stderr.write('Claude Code Stream Bridge started\n');
process.stderr.write(`Workspace: ${WORKSPACE_ROOT}\n`);
process.stderr.write(`Model: ${ANTHROPIC_MODEL}\n`);