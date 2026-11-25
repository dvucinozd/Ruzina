/* The Web Remote server's connections: reading a request and sending its answer. Each step
 * is bounded and never waits, so one slow client holds no other back. Included by
 * wifi_remote.c, and by a host test with fake sockets.
 *
 * The includer defines, before this header:
 *   int web_recv(int sock, uint8_t *buffer, size_t len);       recv with MSG_DONTWAIT
 *   int web_send(int sock, const uint8_t *buffer, size_t len); send with MSG_DONTWAIT
 *   void web_close(int sock);
 * and REQUEST_MAX, the longest request.
 *
 * A body too long for REQUEST_MAX (a firmware bundle) is streamed when the includer also
 * defines WEB_STREAM_MAX, the longest streamed body, and:
 *   int web_stream_open(const uint8_t *head, size_t len);  0 to take the body, else the
 *                                                           HTTP status refusing it
 *   int web_stream_feed(const uint8_t *bytes, size_t len); 0, else the status refusing it
 *   void web_stream_cancel(void);                          the stream ends unfinished
 * The body then goes to web_stream_feed as it arrives, at most REQUEST_MAX bytes at a
 * time, and the request counts as complete once all of it is in. A refused body is still
 * read to its end, so the answer reaches the client: `answer` sees `c->streaming` and
 * `c->stream_status`. */
#ifndef RUZINA_WEB_CLIENT_H
#define RUZINA_WEB_CLIENT_H

#include <ctype.h>
#include <errno.h>
#include <stdbool.h>
#include <stdint.h>
#include <string.h>

/* The most one step hands the socket: a large answer goes in pieces, with the other
 * connections served between them. Two full-size TCP segments. */
#define WEB_SEND_SLICE 2920
/* How long a connection may take to send its request, and then to take its answer. */
#define CLIENT_TIMEOUT_US 2000000
#define SEND_TIMEOUT_US 5000000
/* How long a streamed body may pause, measured from the last bytes that arrived. */
#define STREAM_TIMEOUT_US 10000000
/* The longest Content-Length read. */
#ifdef WEB_STREAM_MAX
#define WEB_BODY_LIMIT WEB_STREAM_MAX
#else
#define WEB_BODY_LIMIT REQUEST_MAX
#endif

struct client {
    int sock;
    /* The request, `len` bytes so far. */
    uint8_t *buffer;
    size_t len;
    /* The whole request's length, once its head is in; 0 before. */
    size_t want;
    int64_t start;
    /* The answer, kept until it is all sent; `out_len` is 0 while the request comes in. */
    uint8_t *out;
    size_t out_len, out_sent;
    int64_t sending_since;
    /* A streamed body: its bytes still to come, and 0 while it is taken or the status that
     * refused it. */
    bool streaming;
    size_t stream_left;
    int stream_status;
};

/* What a step ended in, for the server's counts. */
enum client_event { CLIENT_WAITING, CLIENT_ANSWERED, CLIENT_DROPPED };

static void client_close(struct client *c)
{
#ifdef WEB_STREAM_MAX
    if (c->streaming) {
        c->streaming = false;
        web_stream_cancel();
    }
#endif
    web_close(c->sock);
    c->sock = -1;
    c->out_len = c->out_sent = 0;
}

/* The Content-Length value in a request head of `len` bytes, or 0. Searches only those
 * bytes: the buffer is reused and never NUL-terminated. */
static size_t content_length(const uint8_t *head, size_t len)
{
    static const char field[] = "\r\ncontent-length:";
    const size_t field_len = sizeof field - 1;
    for (size_t at = 0; at + field_len <= len; ++at) {
        size_t i = 0;
        while (i < field_len && tolower(head[at + i]) == field[i]) ++i;
        if (i < field_len) continue;
        size_t value = 0;
        for (size_t j = at + field_len; j < len; ++j) {
            if (head[j] == ' ' || head[j] == '\t') continue;
            if (head[j] < '0' || head[j] > '9') break;
            if (value > WEB_BODY_LIMIT) return SIZE_MAX;
            value = value * 10 + (size_t)(head[j] - '0');
        }
        return value;
    }
    return 0;
}

#ifdef WEB_STREAM_MAX
/* `len` bytes of a streamed body: fed on while it is taken, else read and dropped. 1 once
 * the body is all in. */
static int stream_take(struct client *c, const uint8_t *bytes, size_t len)
{
    if (len > c->stream_left) len = c->stream_left;
    if (len && c->stream_status == 0) c->stream_status = web_stream_feed(bytes, len);
    c->stream_left -= len;
    return c->stream_left == 0 ? 1 : 0;
}

/* The next piece of a streamed body, into the request buffer. */
static int stream_read(struct client *c)
{
    const size_t want = c->stream_left < REQUEST_MAX ? c->stream_left : REQUEST_MAX;
    int got = web_recv(c->sock, c->buffer, want);
    if (got < 0) return errno == EAGAIN || errno == EWOULDBLOCK ? 0 : -1;
    if (got == 0) return -1;
    return stream_take(c, c->buffer, (size_t)got);
}
#endif

/* Reads what arrived on `c`, without waiting: 1 once the request (the head, then a body
 * up to Content-Length, both within REQUEST_MAX, or a streamed body) is complete, -1 to
 * drop the connection, 0 to wait for more. */
static int client_read(struct client *c)
{
#ifdef WEB_STREAM_MAX
    if (c->streaming) return stream_read(c);
#endif
    int got = web_recv(c->sock, c->buffer + c->len, REQUEST_MAX - c->len);
    if (got < 0) return errno == EAGAIN || errno == EWOULDBLOCK ? 0 : -1;
    if (got == 0) return c->want && c->len >= c->want ? 1 : -1;
    c->len += (size_t)got;
    if (c->want == 0) {
        const uint8_t *end = memmem(c->buffer, c->len, "\r\n\r\n", 4);
        if (end == NULL) return c->len >= REQUEST_MAX ? -1 : 0;
        size_t head = (size_t)(end - c->buffer) + 4;
        size_t body = content_length(c->buffer, head);
#ifdef WEB_STREAM_MAX
        if (body > REQUEST_MAX - head && body <= WEB_STREAM_MAX) {
            c->streaming = true;
            c->stream_left = body;
            c->stream_status = web_stream_open(c->buffer, head);
            return stream_take(c, c->buffer + head, c->len - head);
        }
#endif
        if (body > REQUEST_MAX - head) return -1;
        c->want = head + body;
    }
    return c->len >= c->want ? 1 : 0;
}

/* Hands the socket the next piece of `c`'s answer, at most WEB_SEND_SLICE, without
 * waiting: 1 once it is all sent, -1 to drop the connection, 0 to wait until the socket
 * takes more. */
static int client_send(struct client *c)
{
    const size_t left = c->out_len - c->out_sent;
    const int n = web_send(c->sock, c->out + c->out_sent, left < WEB_SEND_SLICE ? left : WEB_SEND_SLICE);
    if (n < 0) return errno == EAGAIN || errno == EWOULDBLOCK ? 0 : -1;
    c->out_sent += (size_t)n;
    return c->out_sent >= c->out_len ? 1 : 0;
}

/* One pass over a connection, given what select reported for it. A complete request is
 * answered at once through `answer`, which writes the answer to `c->out` and returns its
 * length (0 closes without one); its first piece goes out in the same pass. */
static enum client_event client_step(struct client *c, bool readable, bool writable, int64_t now,
                                     size_t (*answer)(struct client *c))
{
    if (c->out_len == 0) {
        if (readable) {
            /* A streamed body's timeout runs from its last bytes. */
            if (c->streaming) c->start = now;
            const int state = client_read(c);
            if (state < 0) {
                client_close(c);
                return CLIENT_DROPPED;
            }
            if (state > 0) {
                c->out_len = answer(c);
                if (c->out_len == 0) {
                    client_close(c);
                    return CLIENT_ANSWERED;
                }
                c->out_sent = 0;
                c->sending_since = now;
                writable = true;
            }
        }
        if (c->out_len == 0) {
            if (now - c->start > (c->streaming ? STREAM_TIMEOUT_US : CLIENT_TIMEOUT_US)) {
                client_close(c);
                return CLIENT_DROPPED;
            }
            return CLIENT_WAITING;
        }
    }
    if (writable) {
        const int sent = client_send(c);
        if (sent < 0) {
            client_close(c);
            return CLIENT_DROPPED;
        }
        if (sent > 0) {
            client_close(c);
            return CLIENT_ANSWERED;
        }
    }
    if (now - c->sending_since > SEND_TIMEOUT_US) {
        client_close(c);
        return CLIENT_DROPPED;
    }
    return CLIENT_WAITING;
}

#endif
