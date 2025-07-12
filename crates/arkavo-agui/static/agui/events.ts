// AG-UI Event Types (TypeScript version)

export type AgUiEvent = 
    | ConnectEvent
    | UserMessageEvent
    | ToolResultEvent
    | UiActionEvent
    | StateSnapshotEvent
    | MessagesSnapshotEvent
    | MessageDeltaEvent
    | ToolCallEvent
    | StateDeltaEvent
    | TypingStartEvent
    | TypingStopEvent
    | LifecycleStartEvent
    | LifecycleEndEvent
    | LifecycleErrorEvent
    | ErrorEvent;

// Outbound events (frontend → agent)
export interface ConnectEvent {
    type: 'connect';
    agentId: string;
    aguiVersion: string;
    sinceEventId?: string;
}

export interface UserMessageEvent {
    type: 'userMessage';
    content: string;
    attachments?: Attachment[];
}

export interface ToolResultEvent {
    type: 'toolResult';
    toolCallId: string;
    result: any;
}

export interface UiActionEvent {
    type: 'uiAction';
    action: string;
    params?: any;
}

// Inbound events (agent → frontend)
export interface StateSnapshotEvent {
    type: 'stateSnapshot';
    state: any;
    eventId: string;
}

export interface MessagesSnapshotEvent {
    type: 'messagesSnapshot';
    messages: Message[];
    eventId: string;
}

export interface MessageDeltaEvent {
    type: 'messageDelta';
    messageId: string;
    delta: MessageDeltaContent;
}

export interface ToolCallEvent {
    type: 'toolCall';
    toolCallId: string;
    toolName: string;
    arguments: any;
}

export interface StateDeltaEvent {
    type: 'stateDelta';
    patch: JsonPatchOp[];
    eventId: string;
}

export interface TypingStartEvent {
    type: 'typingStart';
    messageId: string;
}

export interface TypingStopEvent {
    type: 'typingStop';
    messageId: string;
}

// Lifecycle events
export interface LifecycleStartEvent {
    type: 'lifecycle.start';
    sessionId: string;
}

export interface LifecycleEndEvent {
    type: 'lifecycle.end';
    reason: string;
}

export interface LifecycleErrorEvent {
    type: 'lifecycle.error';
    code: string;
    message: string;
}

export interface ErrorEvent {
    type: 'error';
    code: string;
    message: string;
}

// Supporting types
export interface Message {
    id: string;
    role: 'user' | 'assistant' | 'system';
    content: string;
    toolCalls?: ToolCall[];
    metadata?: Record<string, any>;
}

export interface ToolCall {
    id: string;
    name: string;
    arguments: any;
}

export interface Attachment {
    type: string;
    url: string;
    name?: string;
}

export type MessageDeltaContent = 
    | { type: 'text'; text: string }
    | { type: 'toolCall'; toolCallId: string; delta: string };

// JSON Patch operations (RFC 6902)
export type JsonPatchOp =
    | { op: 'add'; path: string; value: any }
    | { op: 'remove'; path: string }
    | { op: 'replace'; path: string; value: any }
    | { op: 'move'; from: string; path: string }
    | { op: 'copy'; from: string; path: string }
    | { op: 'test'; path: string; value: any };

// Event handlers interface
export interface AgUiEventHandlers {
    onStateSnapshot?: (event: StateSnapshotEvent) => void;
    onMessagesSnapshot?: (event: MessagesSnapshotEvent) => void;
    onMessageDelta?: (event: MessageDeltaEvent) => void;
    onToolCall?: (event: ToolCallEvent) => void;
    onStateDelta?: (event: StateDeltaEvent) => void;
    onTypingStart?: (event: TypingStartEvent) => void;
    onTypingStop?: (event: TypingStopEvent) => void;
    onLifecycleStart?: (event: LifecycleStartEvent) => void;
    onLifecycleEnd?: (event: LifecycleEndEvent) => void;
    onLifecycleError?: (event: LifecycleErrorEvent) => void;
    onError?: (event: ErrorEvent) => void;
}