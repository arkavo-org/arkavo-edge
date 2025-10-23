import {Message, MessageDeltaEvent, MessagesSnapshotEvent} from '../events';

export class MessageHandler {
    private messages: Map<string, Message> = new Map();
    private container: HTMLElement;

    constructor(containerId: string) {
        const container = document.getElementById(containerId);
        if (!container) {
            throw new Error(`Container ${containerId} not found`);
        }
        this.container = container;
    }

    handleSnapshot(event: MessagesSnapshotEvent): void {
        this.messages.clear();
        event.messages.forEach(msg => {
            this.messages.set(msg.id, msg);
        });
        this.render();
    }

    handleDelta(event: MessageDeltaEvent): void {
        const message = this.messages.get(event.messageId);
        if (!message) {
            // Create new message if it doesn't exist
            const newMessage: Message = {
                id: event.messageId,
                role: 'assistant', // Assume assistant for streaming
                content: '',
                metadata: {}
            };
            this.messages.set(event.messageId, newMessage);
        }

        const msg = this.messages.get(event.messageId)!;
        
        if (event.delta.type === 'text') {
            msg.content += event.delta.text;
        } else if (event.delta.type === 'toolCall') {
            // Handle tool call deltas
            if (!msg.toolCalls) {
                msg.toolCalls = [];
            }
            // Update or add tool call
            const toolCall = msg.toolCalls.find(tc => tc.id === event.delta.toolCallId);
            if (toolCall) {
                toolCall.arguments = JSON.parse(event.delta.delta);
            }
        }

        this.renderMessage(event.messageId);
    }

    private render(): void {
        this.container.innerHTML = '';
        this.messages.forEach((msg, id) => {
            this.renderMessage(id);
        });
    }

    private renderMessage(messageId: string): void {
        const message = this.messages.get(messageId);
        if (!message) return;

        let msgEl = document.getElementById(`msg-${messageId}`);
        if (!msgEl) {
            msgEl = document.createElement('div');
            msgEl.id = `msg-${messageId}`;
            msgEl.className = `message message-${message.role}`;
            this.container.appendChild(msgEl);
        }

        msgEl.innerHTML = `
            <div class="message-role">${message.role}</div>
            <div class="message-content">${this.escapeHtml(message.content)}</div>
            ${message.toolCalls ? this.renderToolCalls(message.toolCalls) : ''}
        `;
    }

    private renderToolCalls(toolCalls: any[]): string {
        return `
            <div class="tool-calls">
                ${toolCalls.map(tc => `
                    <div class="tool-call">
                        <span class="tool-name">${tc.name}</span>
                        <pre class="tool-args">${JSON.stringify(tc.arguments, null, 2)}</pre>
                    </div>
                `).join('')}
            </div>
        `;
    }

    private escapeHtml(text: string): string {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
    }

    addUserMessage(content: string): string {
        const id = `user-${Date.now()}`;
        const message: Message = {
            id,
            role: 'user',
            content,
            metadata: { timestamp: new Date().toISOString() }
        };
        this.messages.set(id, message);
        this.renderMessage(id);
        return id;
    }
}