package model

import "time"

// EventKind enumerates normalized cross-transport events.
type EventKind string

const (
	EventText     EventKind = "Text"
	EventAction   EventKind = "Action"
	EventBot      EventKind = "Bot"
	EventDelete   EventKind = "Delete"
	EventPin      EventKind = "Pin"
	EventReaction EventKind = "Reaction"
	EventNames    EventKind = "Names"
)

// ThreadRef carries transport-specific thread linkage.
type ThreadRef struct {
	SlackThreadTS *string `json:"slack_thread_ts,omitempty"`
	DiscordThread *uint64 `json:"discord_thread,omitempty"`
}

// Attachment is a normalized attachment payload for forwarding/edit paths.
type Attachment struct {
	ID           uint64  `json:"id"`
	PipoID       *int64  `json:"pipo_id,omitempty"`
	ServiceName  *string `json:"service_name,omitempty"`
	ServiceURL   *string `json:"service_url,omitempty"`
	AuthorName   *string `json:"author_name,omitempty"`
	AuthorSub    *string `json:"author_subname,omitempty"`
	AuthorLink   *string `json:"author_link,omitempty"`
	AuthorIcon   *string `json:"author_icon,omitempty"`
	Filename     *string `json:"filename,omitempty"`
	FromURL      *string `json:"from_url,omitempty"`
	OriginalURL  *string `json:"original_url,omitempty"`
	Footer       *string `json:"footer,omitempty"`
	FooterIcon   *string `json:"footer_icon,omitempty"`
	ContentType  *string `json:"content_type,omitempty"`
	Size         *uint64 `json:"size,omitempty"`
	Text         *string `json:"text,omitempty"`
	ImageURL     *string `json:"image_url,omitempty"`
	ImageBytes   *uint64 `json:"image_bytes,omitempty"`
	ImageHeight  *uint64 `json:"image_height,omitempty"`
	ImageWidth   *uint64 `json:"image_width,omitempty"`
	FallbackText *string `json:"fallback,omitempty"`
}

// SourceRef identifies the transport/local message behind a normalized event.
type SourceRef struct {
	Transport string  `json:"transport"`
	BusID     string  `json:"bus_id"`
	MessageID *string `json:"message_id,omitempty"`
}

// Event is the normalized equivalent of Rust Message variants, with optional
// fields needed for cross-transport forwarding and edit/reaction synchronization.
type Event struct {
	Kind        EventKind    `json:"kind"`
	Sender      int          `json:"sender"`
	PipoID      *int64       `json:"pipo_id,omitempty"`
	Source      SourceRef    `json:"source"`
	Username    *string      `json:"username,omitempty"`
	AvatarURL   *string      `json:"avatar_url,omitempty"`
	Thread      *ThreadRef   `json:"thread,omitempty"`
	Message     *string      `json:"message,omitempty"`
	Attachments []Attachment `json:"attachments,omitempty"`
	IsEdit      bool         `json:"is_edit,omitempty"`
	IRCFlag     bool         `json:"irc_flag,omitempty"`
	Emoji       *string      `json:"emoji,omitempty"`
	Remove      bool         `json:"remove,omitempty"`
	CreatedAt   time.Time    `json:"created_at"`
}
