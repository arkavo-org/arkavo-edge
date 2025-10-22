import {AgUiEvent, AgUiEventHandlers} from './events';

export class AgUiWebSocket {
    private ws: WebSocket | null = null;
    private url: string;
    private reconnectAttempts = 0;
    private maxReconnectAttempts = 10;
    private reconnectDelay = 500; // Start with 500ms
    private maxReconnectDelay = 8000; // Max 8 seconds
    private eventHandlers: AgUiEventHandlers;
    private isConnecting = false;
    private sinceEventId?: string;
    private heartbeatInterval?: number;

    constructor(url: string, handlers: AgUiEventHandlers) {
        this.url = url;
        this.eventHandlers = handlers;
    }

    connect(agentId: string): void {
        if (this.isConnecting || (this.ws && this.ws.readyState === WebSocket.OPEN)) {
            return;
        }

        this.isConnecting = true;

        try {
            this.ws = new WebSocket(this.url);
            
            this.ws.onopen = () => {
                console.log('AG-UI WebSocket connected');
                this.isConnecting = false;
                this.reconnectAttempts = 0;
                this.reconnectDelay = 500;
                
                // Send connect event
                this.send({
                    type: 'connect',
                    agentId: agentId,
                    aguiVersion: '1.0',
                    sinceEventId: this.sinceEventId
                });

                // Start heartbeat
                this.startHeartbeat();
            };

            this.ws.onmessage = (event) => {
                try {
                    const aguiEvent = JSON.parse(event.data) as AgUiEvent;
                    this.handleEvent(aguiEvent);
                } catch (error) {
                    console.error('Failed to parse AG-UI event:', error);
                }
            };

            this.ws.onerror = (error) => {
                console.error('WebSocket error:', error);
                this.isConnecting = false;
            };

            this.ws.onclose = (event) => {
                console.log('WebSocket closed:', event.code, event.reason);
                this.isConnecting = false;
                this.stopHeartbeat();
                
                // Handle reconnection
                if (event.code !== 1000) { // Not a normal closure
                    this.reconnect(agentId);
                }
            };
        } catch (error) {
            console.error('Failed to create WebSocket:', error);
            this.isConnecting = false;
            this.reconnect(agentId);
        }
    }

    private reconnect(agentId: string): void {
        if (this.reconnectAttempts >= this.maxReconnectAttempts) {
            console.error('Max reconnection attempts reached');
            if (this.eventHandlers.onError) {
                this.eventHandlers.onError({
                    type: 'error',
                    code: 'MAX_RECONNECT',
                    message: 'Maximum reconnection attempts reached'
                });
            }
            return;
        }

        this.reconnectAttempts++;
        const delay = Math.min(this.reconnectDelay * Math.pow(2, this.reconnectAttempts - 1), this.maxReconnectDelay);
        
        console.log(`Reconnecting in ${delay}ms (attempt ${this.reconnectAttempts})`);
        
        setTimeout(() => {
            this.connect(agentId);
        }, delay);
    }

    send(event: AgUiEvent): void {
        if (this.ws && this.ws.readyState === WebSocket.OPEN) {
            this.ws.send(JSON.stringify(event));
        } else {
            console.error('WebSocket is not connected');
        }
    }

    close(): void {
        this.stopHeartbeat();
        if (this.ws) {
            this.ws.close(1000, 'Normal closure');
            this.ws = null;
        }
    }

    private handleEvent(event: AgUiEvent): void {
        // Track event IDs for reconnection
        if ('eventId' in event && event.eventId) {
            this.sinceEventId = event.eventId;
        }

        // Route to appropriate handler
        switch (event.type) {
            case 'stateSnapshot':
                if (this.eventHandlers.onStateSnapshot) {
                    this.eventHandlers.onStateSnapshot(event);
                }
                break;
            case 'messagesSnapshot':
                if (this.eventHandlers.onMessagesSnapshot) {
                    this.eventHandlers.onMessagesSnapshot(event);
                }
                break;
            case 'messageDelta':
                if (this.eventHandlers.onMessageDelta) {
                    this.eventHandlers.onMessageDelta(event);
                }
                break;
            case 'toolCall':
                if (this.eventHandlers.onToolCall) {
                    this.eventHandlers.onToolCall(event);
                }
                break;
            case 'stateDelta':
                if (this.eventHandlers.onStateDelta) {
                    this.eventHandlers.onStateDelta(event);
                }
                break;
            case 'typingStart':
                if (this.eventHandlers.onTypingStart) {
                    this.eventHandlers.onTypingStart(event);
                }
                break;
            case 'typingStop':
                if (this.eventHandlers.onTypingStop) {
                    this.eventHandlers.onTypingStop(event);
                }
                break;
            case 'lifecycle.start':
                if (this.eventHandlers.onLifecycleStart) {
                    this.eventHandlers.onLifecycleStart(event);
                }
                break;
            case 'lifecycle.end':
                if (this.eventHandlers.onLifecycleEnd) {
                    this.eventHandlers.onLifecycleEnd(event);
                }
                break;
            case 'lifecycle.error':
                if (this.eventHandlers.onLifecycleError) {
                    this.eventHandlers.onLifecycleError(event);
                }
                break;
            case 'error':
                if (this.eventHandlers.onError) {
                    this.eventHandlers.onError(event);
                }
                break;
            default:
                console.warn('Unknown AG-UI event type:', event);
        }
    }

    private startHeartbeat(): void {
        this.heartbeatInterval = window.setInterval(() => {
            if (this.ws && this.ws.readyState === WebSocket.OPEN) {
                // Send a ping frame (WebSocket will handle this automatically)
                // Just check the connection is alive
            } else {
                this.stopHeartbeat();
            }
        }, 30000); // Every 30 seconds
    }

    private stopHeartbeat(): void {
        if (this.heartbeatInterval) {
            clearInterval(this.heartbeatInterval);
            this.heartbeatInterval = undefined;
        }
    }
}