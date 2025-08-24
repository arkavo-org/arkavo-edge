#!/usr/bin/env node

/**
 * Claude Code SDK Bridge for Arkavo
 * Provides JSON-RPC interface to Claude Code SDK
 */

const readline = require('readline');

// Try to load Claude Code SDK from multiple locations
let ClaudeCodeSession;
const path = require('path');
const possiblePaths = [
    // Local Claude installation (preferred - standard location)
    path.join(process.env.HOME, '.claude', 'local', 'node_modules', '@anthropic-ai', 'claude-code'),
    // Global npm installation
    '@anthropic-ai/claude-code',
    // Local node_modules (relative to this script)
    './node_modules/@anthropic-ai/claude-code',
    // Alternative global locations
    '/usr/local/lib/node_modules/@anthropic-ai/claude-code',
    '/opt/homebrew/lib/node_modules/@anthropic-ai/claude-code',
];

let loadedFrom = null;
for (const modulePath of possiblePaths) {
    try {
        const module = require(modulePath);
        ClaudeCodeSession = module.ClaudeCodeSession;
        loadedFrom = modulePath;
        // Log to stderr so it doesn't interfere with JSON-RPC
        process.stderr.write(`Claude Code SDK loaded from: ${modulePath}\n`);
        break;
    } catch (error) {
        // Try next path
    }
}

if (!ClaudeCodeSession) {
    // Send error notification and exit
    console.log(JSON.stringify({
        jsonrpc: '2.0',
        method: 'error',
        params: {
            error: 'Claude Code SDK not found. Please install it:\n1. Via Claude CLI (recommended): Install from https://claude.ai/\n2. Via npm: npm install -g @anthropic-ai/claude-code',
            code: 'SDK_NOT_FOUND'
        }
    }));
    process.exit(1);
}

// Configuration from environment
const ANTHROPIC_API_KEY = process.env.ANTHROPIC_API_KEY;
const ANTHROPIC_AUTH_TOKEN = process.env.ANTHROPIC_AUTH_TOKEN;
const ANTHROPIC_BASE_URL = process.env.ANTHROPIC_BASE_URL;
const ANTHROPIC_MODEL = process.env.ANTHROPIC_MODEL || 'claude-3-sonnet-20240229';
const ANTHROPIC_SMALL_FAST_MODEL = process.env.ANTHROPIC_SMALL_FAST_MODEL;

// Active sessions map
const sessions = new Map();

// JSON-RPC helper functions
function sendResponse(id, result) {
    const response = {
        jsonrpc: '2.0',
        id,
        result
    };
    console.log(JSON.stringify(response));
}

function sendError(id, code, message, data = null) {
    const response = {
        jsonrpc: '2.0',
        id,
        error: {
            code,
            message,
            data
        }
    };
    console.log(JSON.stringify(response));
}

function sendNotification(method, params) {
    const notification = {
        jsonrpc: '2.0',
        method,
        params
    };
    console.log(JSON.stringify(notification));
}

// Tool interceptor for policy enforcement
async function interceptTool(toolRequest) {
    return new Promise((resolve) => {
        // Send tool request to Rust for policy check
        sendNotification('tool_request', toolRequest);
        
        // Set up listener for approval/denial
        // For now, approve all (will be replaced with actual policy response)
        // TODO: Implement bidirectional tool approval mechanism
        setTimeout(() => resolve(true), 100);
    });
}

// Event handler for SDK events
function handleSdkEvent(eventType, eventData) {
    sendNotification('event', {
        type: eventType,
        data: eventData,
        timestamp: new Date().toISOString()
    });
}

// RPC method handlers
const rpcHandlers = {
    async initialize(params) {
        // Validate authentication
        if (!ANTHROPIC_API_KEY && !ANTHROPIC_AUTH_TOKEN) {
            throw new Error('Neither ANTHROPIC_API_KEY nor ANTHROPIC_AUTH_TOKEN is set');
        }
        
        return {
            success: true,
            model: ANTHROPIC_MODEL,
            baseUrl: ANTHROPIC_BASE_URL || 'https://api.anthropic.com',
            version: '1.0.0'
        };
    },
    
    async openSession(params) {
        const { context } = params;
        const sessionId = `session_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
        
        try {
            // Create session configuration
            const sessionConfig = {
                apiKey: ANTHROPIC_API_KEY || ANTHROPIC_AUTH_TOKEN,
                model: ANTHROPIC_MODEL,
                context: context || {},
            };
            
            // Add base URL if provided (for DeepSeek, etc.)
            if (ANTHROPIC_BASE_URL) {
                sessionConfig.baseURL = ANTHROPIC_BASE_URL;
            }
            
            // Add small/fast model if provided
            if (ANTHROPIC_SMALL_FAST_MODEL) {
                sessionConfig.smallFastModel = ANTHROPIC_SMALL_FAST_MODEL;
            }
            
            // Event handlers
            sessionConfig.onPlanUpdate = (plan) => {
                handleSdkEvent('plan_updated', { sessionId, plan });
            };
            
            sessionConfig.onToolUse = async (tool) => {
                handleSdkEvent('tool_use', { sessionId, tool });
                
                // Check policy
                const approved = await interceptTool(tool);
                if (!approved) {
                    throw new Error(`Tool use denied by policy: ${tool.name}`);
                }
                
                return true;
            };
            
            sessionConfig.onToolResult = (result) => {
                handleSdkEvent('tool_result', { sessionId, result });
            };
            
            sessionConfig.onContent = (content) => {
                handleSdkEvent('content_chunk', { sessionId, content });
            };
            
            sessionConfig.onError = (error) => {
                handleSdkEvent('error', { sessionId, error: error.message });
            };
            
            sessionConfig.onTokenUsage = (usage) => {
                handleSdkEvent('token_usage', { sessionId, usage });
            };
            
            const session = new ClaudeCodeSession(sessionConfig);
            
            await session.initialize();
            sessions.set(sessionId, session);
            
            return { sessionId };
        } catch (error) {
            throw new Error(`Failed to open session: ${error.message}`);
        }
    },
    
    async streamRun(params) {
        const { sessionId, prompt } = params;
        
        const session = sessions.get(sessionId);
        if (!session) {
            throw new Error(`Session not found: ${sessionId}`);
        }
        
        try {
            // Start streaming run
            handleSdkEvent('run_started', { sessionId, prompt });
            
            // Execute the task
            await session.run(prompt, {
                stream: true,
                maxTokens: 100000, // Will be limited by budget tracker
                
                // Tool configuration
                tools: {
                    file_read: true,
                    file_write: process.env.ALLOW_WRITE === 'true',
                    shell_exec: process.env.ALLOW_EXEC === 'true',
                    web_search: process.env.ALLOW_WEB === 'true'
                }
            });
            
            handleSdkEvent('run_completed', { sessionId });
            
            return { success: true };
        } catch (error) {
            handleSdkEvent('run_failed', { sessionId, error: error.message });
            throw new Error(`Run failed: ${error.message}`);
        }
    },
    
    async closeSession(params) {
        const { sessionId } = params;
        
        const session = sessions.get(sessionId);
        if (!session) {
            return { success: true }; // Already closed
        }
        
        try {
            await session.close();
            sessions.delete(sessionId);
            
            handleSdkEvent('session_closed', { sessionId });
            
            return { success: true };
        } catch (error) {
            throw new Error(`Failed to close session: ${error.message}`);
        }
    },
    
    async listSessions() {
        return {
            sessions: Array.from(sessions.keys())
        };
    }
};

// Main message handler
async function handleMessage(message) {
    try {
        const request = JSON.parse(message);
        
        if (!request.jsonrpc || request.jsonrpc !== '2.0') {
            sendError(request.id, -32600, 'Invalid JSON-RPC version');
            return;
        }
        
        const handler = rpcHandlers[request.method];
        if (!handler) {
            sendError(request.id, -32601, `Method not found: ${request.method}`);
            return;
        }
        
        try {
            const result = await handler(request.params || {});
            sendResponse(request.id, result);
        } catch (error) {
            sendError(request.id, -32000, error.message);
        }
    } catch (error) {
        sendError(null, -32700, 'Parse error', error.message);
    }
}

// Set up stdin reader
const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
    terminal: false
});

rl.on('line', handleMessage);

// Handle process termination
process.on('SIGTERM', async () => {
    // Close all sessions
    for (const [sessionId, session] of sessions.entries()) {
        try {
            await session.close();
        } catch (error) {
            console.error(`Error closing session ${sessionId}:`, error);
        }
    }
    process.exit(0);
});

process.on('SIGINT', async () => {
    process.exit(0);
});

// Send ready notification
sendNotification('ready', {
    model: ANTHROPIC_MODEL,
    timestamp: new Date().toISOString()
});

// Log to stderr for debugging
console.error('Claude Code Bridge started');
console.error(`Model: ${ANTHROPIC_MODEL}`);
console.error(`API Key: ${ANTHROPIC_API_KEY ? 'Set' : 'Not set'}`);