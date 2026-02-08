import React, {
  useState,
  useEffect,
  useCallback,
  useReducer,
  useRef,
  forwardRef,
  useImperativeHandle,
  createContext,
  useContext,
} from "react";

// --- Type Definitions ---

interface User {
  id: string;
  name: string;
  email: string;
  avatar: string;
  role: "admin" | "editor" | "viewer";
  lastActive: Date;
}

interface Message {
  id: string;
  senderId: string;
  content: string;
  timestamp: Date;
  edited: boolean;
  reactions: Record<string, string[]>;
  attachments: Attachment[];
  replyTo?: string;
  status: "sending" | "sent" | "delivered" | "read" | "failed";
}

interface Attachment {
  id: string;
  name: string;
  type: "image" | "file" | "link";
  url: string;
  size: number;
  thumbnailUrl?: string;
}

interface Channel {
  id: string;
  name: string;
  description: string;
  members: string[];
  isPrivate: boolean;
  unreadCount: number;
  lastMessage?: Message;
  pinnedMessages: string[];
}

interface TypingIndicator {
  userId: string;
  channelId: string;
  expiresAt: number;
}

// --- State Management ---

type ChatAction =
  | { type: "SET_CHANNEL"; channelId: string }
  | { type: "ADD_MESSAGE"; message: Message }
  | { type: "EDIT_MESSAGE"; messageId: string; content: string }
  | { type: "DELETE_MESSAGE"; messageId: string }
  | { type: "ADD_REACTION"; messageId: string; emoji: string; userId: string }
  | { type: "REMOVE_REACTION"; messageId: string; emoji: string; userId: string }
  | { type: "SET_TYPING"; indicator: TypingIndicator }
  | { type: "CLEAR_TYPING"; userId: string; channelId: string }
  | { type: "MARK_READ"; channelId: string; messageId: string }
  | { type: "SET_MESSAGES"; channelId: string; messages: Message[] }
  | { type: "SET_SEARCH_RESULTS"; results: Message[] }
  | { type: "CLEAR_SEARCH" };

interface ChatState {
  activeChannelId: string | null;
  messagesByChannel: Record<string, Message[]>;
  typingIndicators: TypingIndicator[];
  searchResults: Message[] | null;
}

function chatReducer(state: ChatState, action: ChatAction): ChatState {
  switch (action.type) {
    case "SET_CHANNEL":
      return { ...state, activeChannelId: action.channelId, searchResults: null };

    case "ADD_MESSAGE": {
      const channelId = state.activeChannelId;
      if (!channelId) return state;
      const existing = state.messagesByChannel[channelId] ?? [];
      return {
        ...state,
        messagesByChannel: {
          ...state.messagesByChannel,
          [channelId]: [...existing, action.message],
        },
      };
    }

    case "EDIT_MESSAGE": {
      const channelId = state.activeChannelId;
      if (!channelId) return state;
      const messages = (state.messagesByChannel[channelId] ?? []).map((msg) =>
        msg.id === action.messageId ? { ...msg, content: action.content, edited: true } : msg
      );
      return {
        ...state,
        messagesByChannel: { ...state.messagesByChannel, [channelId]: messages },
      };
    }

    case "DELETE_MESSAGE": {
      const channelId = state.activeChannelId;
      if (!channelId) return state;
      const messages = (state.messagesByChannel[channelId] ?? []).filter(
        (msg) => msg.id !== action.messageId
      );
      return {
        ...state,
        messagesByChannel: { ...state.messagesByChannel, [channelId]: messages },
      };
    }

    case "ADD_REACTION": {
      const channelId = state.activeChannelId;
      if (!channelId) return state;
      const messages = (state.messagesByChannel[channelId] ?? []).map((msg) => {
        if (msg.id !== action.messageId) return msg;
        const users = msg.reactions[action.emoji] ?? [];
        if (users.includes(action.userId)) return msg;
        return {
          ...msg,
          reactions: { ...msg.reactions, [action.emoji]: [...users, action.userId] },
        };
      });
      return {
        ...state,
        messagesByChannel: { ...state.messagesByChannel, [channelId]: messages },
      };
    }

    case "REMOVE_REACTION": {
      const channelId = state.activeChannelId;
      if (!channelId) return state;
      const messages = (state.messagesByChannel[channelId] ?? []).map((msg) => {
        if (msg.id !== action.messageId) return msg;
        const users = (msg.reactions[action.emoji] ?? []).filter((id) => id !== action.userId);
        const reactions = { ...msg.reactions };
        if (users.length === 0) {
          delete reactions[action.emoji];
        } else {
          reactions[action.emoji] = users;
        }
        return { ...msg, reactions };
      });
      return {
        ...state,
        messagesByChannel: { ...state.messagesByChannel, [channelId]: messages },
      };
    }

    case "SET_TYPING":
      return {
        ...state,
        typingIndicators: [
          ...state.typingIndicators.filter(
            (t) => !(t.userId === action.indicator.userId && t.channelId === action.indicator.channelId)
          ),
          action.indicator,
        ],
      };

    case "CLEAR_TYPING":
      return {
        ...state,
        typingIndicators: state.typingIndicators.filter(
          (t) => !(t.userId === action.userId && t.channelId === action.channelId)
        ),
      };

    case "SET_MESSAGES":
      return {
        ...state,
        messagesByChannel: { ...state.messagesByChannel, [action.channelId]: action.messages },
      };

    case "SET_SEARCH_RESULTS":
      return { ...state, searchResults: action.results };

    case "CLEAR_SEARCH":
      return { ...state, searchResults: null };

    default:
      return state;
  }
}

// --- Context ---

interface ChatContextValue {
  state: ChatState;
  dispatch: React.Dispatch<ChatAction>;
  currentUser: User;
  users: Map<string, User>;
}

const ChatContext = createContext<ChatContextValue | null>(null);

function useChatContext(): ChatContextValue {
  const ctx = useContext(ChatContext);
  if (!ctx) throw new Error("useChatContext must be used within ChatProvider");
  return ctx;
}

// --- Components ---

interface MessageBubbleProps {
  message: Message;
  isOwn: boolean;
  showAvatar: boolean;
  onReply: (messageId: string) => void;
  onEdit: (messageId: string, content: string) => void;
  onDelete: (messageId: string) => void;
}

const MessageBubble = React.memo<MessageBubbleProps>(
  ({ message, isOwn, showAvatar, onReply, onEdit, onDelete }) => {
    const { users, currentUser, dispatch } = useChatContext();
    const [editing, setEditing] = useState(false);
    const [editContent, setEditContent] = useState(message.content);
    const [showActions, setShowActions] = useState(false);
    const sender = users.get(message.senderId);

    const handleReaction = useCallback(
      (emoji: string) => {
        const existing = message.reactions[emoji] ?? [];
        if (existing.includes(currentUser.id)) {
          dispatch({ type: "REMOVE_REACTION", messageId: message.id, emoji, userId: currentUser.id });
        } else {
          dispatch({ type: "ADD_REACTION", messageId: message.id, emoji, userId: currentUser.id });
        }
      },
      [message, currentUser.id, dispatch]
    );

    const handleSaveEdit = () => {
      if (editContent.trim() && editContent !== message.content) {
        onEdit(message.id, editContent.trim());
      }
      setEditing(false);
    };

    const statusIcon: Record<Message["status"], string> = {
      sending: "\u23F3",
      sent: "\u2713",
      delivered: "\u2713\u2713",
      read: "\u2713\u2713",
      failed: "\u26A0",
    };

    return (
      <div
        className={`message ${isOwn ? "message--own" : ""}`}
        onMouseEnter={() => setShowActions(true)}
        onMouseLeave={() => setShowActions(false)}
      >
        {showAvatar && !isOwn && (
          <img src={sender?.avatar} alt={sender?.name} className="message__avatar" />
        )}

        <div className="message__body">
          {showAvatar && !isOwn && (
            <span className="message__sender">{sender?.name ?? "Unknown"}</span>
          )}

          {editing ? (
            <div className="message__edit">
              <textarea
                value={editContent}
                onChange={(e) => setEditContent(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    handleSaveEdit();
                  }
                  if (e.key === "Escape") setEditing(false);
                }}
                autoFocus
              />
              <div className="message__edit-actions">
                <button onClick={handleSaveEdit}>Save</button>
                <button onClick={() => setEditing(false)}>Cancel</button>
              </div>
            </div>
          ) : (
            <div className="message__content">
              <p>{message.content}</p>
              {message.edited && <span className="message__edited">(edited)</span>}
            </div>
          )}

          {message.attachments.length > 0 && (
            <div className="message__attachments">
              {message.attachments.map((att) => (
                <AttachmentPreview key={att.id} attachment={att} />
              ))}
            </div>
          )}

          {Object.keys(message.reactions).length > 0 && (
            <div className="message__reactions">
              {Object.entries(message.reactions).map(([emoji, userIds]) => (
                <button
                  key={emoji}
                  className={`reaction ${userIds.includes(currentUser.id) ? "reaction--active" : ""}`}
                  onClick={() => handleReaction(emoji)}
                >
                  {emoji} {userIds.length}
                </button>
              ))}
            </div>
          )}

          <div className="message__meta">
            <time dateTime={message.timestamp.toISOString()}>
              {formatTime(message.timestamp)}
            </time>
            {isOwn && <span className={`status status--${message.status}`}>{statusIcon[message.status]}</span>}
          </div>
        </div>

        {showActions && (
          <div className="message__actions">
            <button onClick={() => onReply(message.id)} title="Reply">
              &#8617;
            </button>
            <button onClick={() => handleReaction("\uD83D\uDC4D")} title="Like">
              &#128077;
            </button>
            {isOwn && (
              <>
                <button onClick={() => setEditing(true)} title="Edit">
                  &#9998;
                </button>
                <button onClick={() => onDelete(message.id)} title="Delete">
                  &#128465;
                </button>
              </>
            )}
          </div>
        )}
      </div>
    );
  }
);
MessageBubble.displayName = "MessageBubble";

function AttachmentPreview({ attachment }: { attachment: Attachment }) {
  if (attachment.type === "image") {
    return (
      <a href={attachment.url} target="_blank" rel="noopener noreferrer" className="attachment--image">
        <img
          src={attachment.thumbnailUrl ?? attachment.url}
          alt={attachment.name}
          loading="lazy"
        />
      </a>
    );
  }

  return (
    <a href={attachment.url} download className="attachment--file">
      <span className="attachment__icon">&#128196;</span>
      <span className="attachment__info">
        <span className="attachment__name">{attachment.name}</span>
        <span className="attachment__size">{formatSize(attachment.size)}</span>
      </span>
    </a>
  );
}

// --- Composer with ref handle ---

interface ComposerHandle {
  focus: () => void;
  insertText: (text: string) => void;
}

interface ComposerProps {
  onSend: (content: string, attachments: File[]) => void;
  replyTo?: Message | null;
  onCancelReply: () => void;
}

const Composer = forwardRef<ComposerHandle, ComposerProps>(({ onSend, replyTo, onCancelReply }, ref) => {
  const [content, setContent] = useState("");
  const [attachments, setAttachments] = useState<File[]>([]);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  useImperativeHandle(ref, () => ({
    focus: () => textareaRef.current?.focus(),
    insertText: (text: string) => {
      setContent((prev) => prev + text);
      textareaRef.current?.focus();
    },
  }));

  const handleSubmit = () => {
    const trimmed = content.trim();
    if (!trimmed && attachments.length === 0) return;
    onSend(trimmed, attachments);
    setContent("");
    setAttachments([]);
  };

  const handleFileSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(e.target.files ?? []);
    const validFiles = files.filter((f) => f.size <= 25 * 1024 * 1024); // 25MB limit
    setAttachments((prev) => [...prev, ...validFiles]);
  };

  return (
    <div className="composer">
      {replyTo && (
        <div className="composer__reply-bar">
          <span>Replying to: {replyTo.content.slice(0, 80)}</span>
          <button onClick={onCancelReply}>&times;</button>
        </div>
      )}

      {attachments.length > 0 && (
        <div className="composer__attachments">
          {attachments.map((file, idx) => (
            <div key={idx} className="composer__attachment-chip">
              <span>{file.name}</span>
              <button onClick={() => setAttachments((prev) => prev.filter((_, i) => i !== idx))}>
                &times;
              </button>
            </div>
          ))}
        </div>
      )}

      <div className="composer__input-row">
        <button className="composer__attach" onClick={() => fileInputRef.current?.click()}>
          &#128206;
        </button>
        <input
          type="file"
          ref={fileInputRef}
          onChange={handleFileSelect}
          multiple
          hidden
        />
        <textarea
          ref={textareaRef}
          value={content}
          onChange={(e) => setContent(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              handleSubmit();
            }
          }}
          placeholder="Type a message..."
          rows={1}
        />
        <button
          className="composer__send"
          onClick={handleSubmit}
          disabled={!content.trim() && attachments.length === 0}
        >
          Send
        </button>
      </div>
    </div>
  );
});
Composer.displayName = "Composer";

// --- Utilities ---

function formatTime(date: Date): string {
  const now = new Date();
  const diff = now.getTime() - date.getTime();
  if (diff < 60_000) return "now";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return date.toLocaleDateString();
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1_048_576) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

// --- Main Chat App ---

export default function ChatApp(): React.ReactElement {
  const [currentUser] = useState<User>({
    id: "usr_self",
    name: "You",
    email: "you@example.com",
    avatar: "/avatars/self.png",
    role: "admin",
    lastActive: new Date(),
  });

  const [users] = useState(
    () =>
      new Map<string, User>([
        ["usr_self", currentUser],
        [
          "usr_alice",
          {
            id: "usr_alice",
            name: "Alice Chen",
            email: "alice@example.com",
            avatar: "/avatars/alice.png",
            role: "editor",
            lastActive: new Date(),
          },
        ],
        [
          "usr_bob",
          {
            id: "usr_bob",
            name: "Bob Kim",
            email: "bob@example.com",
            avatar: "/avatars/bob.png",
            role: "viewer",
            lastActive: new Date(Date.now() - 3_600_000),
          },
        ],
      ])
  );

  const [state, dispatch] = useReducer(chatReducer, {
    activeChannelId: "ch_general",
    messagesByChannel: {},
    typingIndicators: [],
    searchResults: null,
  });

  const composerRef = useRef<ComposerHandle>(null);
  const [replyTo, setReplyTo] = useState<Message | null>(null);

  const handleSend = useCallback(
    (content: string, _attachments: File[]) => {
      const message: Message = {
        id: `msg_${Date.now()}`,
        senderId: currentUser.id,
        content,
        timestamp: new Date(),
        edited: false,
        reactions: {},
        attachments: [],
        replyTo: replyTo?.id,
        status: "sending",
      };
      dispatch({ type: "ADD_MESSAGE", message });
      setReplyTo(null);
    },
    [currentUser.id, replyTo]
  );

  useEffect(() => {
    composerRef.current?.focus();
  }, [state.activeChannelId]);

  const messages = state.activeChannelId
    ? (state.messagesByChannel[state.activeChannelId] ?? [])
    : [];

  return (
    <ChatContext.Provider value={{ state, dispatch, currentUser, users }}>
      <div className="chat-app">
        <aside className="channel-list">
          {["ch_general", "ch_engineering", "ch_design"].map((chId) => (
            <button
              key={chId}
              className={`channel ${state.activeChannelId === chId ? "channel--active" : ""}`}
              onClick={() => dispatch({ type: "SET_CHANNEL", channelId: chId })}
            >
              # {chId.replace("ch_", "")}
            </button>
          ))}
        </aside>

        <main className="chat-main">
          <div className="message-list">
            {messages.map((msg, idx) => {
              const prev = messages[idx - 1];
              const showAvatar = !prev || prev.senderId !== msg.senderId;
              return (
                <MessageBubble
                  key={msg.id}
                  message={msg}
                  isOwn={msg.senderId === currentUser.id}
                  showAvatar={showAvatar}
                  onReply={(id) => {
                    const m = messages.find((x) => x.id === id);
                    if (m) setReplyTo(m);
                    composerRef.current?.focus();
                  }}
                  onEdit={(id, content) => dispatch({ type: "EDIT_MESSAGE", messageId: id, content })}
                  onDelete={(id) => dispatch({ type: "DELETE_MESSAGE", messageId: id })}
                />
              );
            })}
          </div>

          <Composer
            ref={composerRef}
            onSend={handleSend}
            replyTo={replyTo}
            onCancelReply={() => setReplyTo(null)}
          />
        </main>
      </div>
    </ChatContext.Provider>
  );
}
