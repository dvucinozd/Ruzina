/* The Web Remote server's connections (main/web_client.h) over fake sockets: a large
 * answer goes out in pieces as its socket takes them, and a small request from another
 * client is answered meanwhile. */
#define _GNU_SOURCE
#include <assert.h>
#include <errno.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#define REQUEST_MAX 2048

/* A fake socket: the request it delivers, how much it takes per send, and what it got. */
static struct sock {
    const char *request;
    size_t request_at;
    size_t takes;   /* bytes one send may hand over; 0 means it is full (EAGAIN) */
    int fail;       /* errno for its next send, or 0 */
    size_t got;     /* bytes sent to it */
    int sends;      /* send calls */
    size_t largest; /* the largest one send asked to hand over */
    bool closed;
} socks[4];

static int web_recv(int sock, uint8_t *buffer, size_t len)
{
    struct sock *s = &socks[sock];
    const size_t left = strlen(s->request) - s->request_at;
    if (left == 0) {
        errno = EAGAIN;
        return -1;
    }
    const size_t n = left < len ? left : len;
    memcpy(buffer, s->request + s->request_at, n);
    s->request_at += n;
    return (int)n;
}

static int web_send(int sock, const uint8_t *buffer, size_t len)
{
    struct sock *s = &socks[sock];
    (void)buffer;
    s->sends++;
    if (len > s->largest) s->largest = len;
    if (s->fail) {
        errno = s->fail;
        return -1;
    }
    if (s->takes == 0) {
        errno = EAGAIN;
        return -1;
    }
    const size_t n = len < s->takes ? len : s->takes;
    s->got += n;
    return (int)n;
}

static void web_close(int sock) { socks[sock].closed = true; }

#include "web_client.h"

/* Answers: the page asset is 12 000 bytes, anything else 200. */
#define ASSET_BYTES 12000
#define CONTROL_BYTES 200
static uint8_t outs[4][ASSET_BYTES];
static size_t answer(struct client *c)
{
    return memmem(c->buffer, c->len, "GET /app.js", 11) ? ASSET_BYTES : CONTROL_BYTES;
}

static uint8_t requests[4][REQUEST_MAX];
static struct client open_client(int sock, const char *request, size_t takes, int64_t now)
{
    socks[sock] = (struct sock){.request = request, .takes = takes};
    return (struct client){.sock = sock, .buffer = requests[sock], .out = outs[sock], .start = now};
}

int main(void)
{
    /* A slow client fetches the largest asset: its socket takes 1000 bytes a pass. A
     * second client's control arrives in the same pass. */
    struct client slow = open_client(0, "GET /app.js HTTP/1.1\r\nHost: 192.168.4.1\r\n\r\n", 1000, 0);
    struct client quick = open_client(
        1, "POST /api/control?action=play_pause&deck=1 HTTP/1.1\r\nContent-Length: 2\r\n\r\nok", 4096, 0);
    int64_t now = 0;
    assert(client_step(&slow, true, false, now, answer) == CLIENT_WAITING);
    assert(slow.out_len == ASSET_BYTES && socks[0].got == 1000 && socks[0].sends == 1);
    assert(client_step(&quick, true, false, now, answer) == CLIENT_ANSWERED);
    assert(socks[1].got == CONTROL_BYTES && socks[1].closed);
    printf("web_client_test control_answered_while_asset_sent=%zu/%d\n", socks[0].got, ASSET_BYTES);
    /* The asset goes on one piece a pass, never more than a slice, until it is all sent. */
    int passes = 1;
    enum client_event event = CLIENT_WAITING;
    while (event == CLIENT_WAITING) {
        now += 1000;
        const int sends = socks[0].sends;
        event = client_step(&slow, false, true, now, answer);
        assert(socks[0].sends == sends + 1);
        passes++;
    }
    assert(event == CLIENT_ANSWERED && socks[0].got == ASSET_BYTES && socks[0].closed);
    assert(passes == 12 && socks[0].largest <= WEB_SEND_SLICE);

    /* A socket that takes everything still gets the asset in slices. */
    struct client fast = open_client(2, "GET /app.js HTTP/1.1\r\n\r\n", 1u << 20, 0);
    assert(client_step(&fast, true, false, 0, answer) == CLIENT_WAITING);
    assert(socks[2].got == WEB_SEND_SLICE);
    while (client_step(&fast, false, true, 0, answer) == CLIENT_WAITING) {}
    assert(socks[2].got == ASSET_BYTES && socks[2].largest == WEB_SEND_SLICE);

    /* A full socket: the step does not wait. A client that stops taking its answer is
     * dropped after SEND_TIMEOUT_US, and one whose send fails at once. */
    struct client stalled = open_client(3, "GET /app.js HTTP/1.1\r\n\r\n", 0, 0);
    assert(client_step(&stalled, true, false, 0, answer) == CLIENT_WAITING);
    assert(stalled.out_len == ASSET_BYTES && stalled.out_sent == 0);
    assert(client_step(&stalled, false, true, SEND_TIMEOUT_US, answer) == CLIENT_WAITING);
    assert(client_step(&stalled, false, false, SEND_TIMEOUT_US + 1, answer) == CLIENT_DROPPED);
    assert(socks[3].closed && stalled.sock == -1 && stalled.out_len == 0);
    struct client broken = open_client(3, "GET /app.js HTTP/1.1\r\n\r\n", 1000, 0);
    socks[3].fail = ECONNRESET;
    assert(client_step(&broken, true, false, 0, answer) == CLIENT_DROPPED && socks[3].closed);

    /* A request that never completes is dropped after CLIENT_TIMEOUT_US. */
    struct client partial = open_client(0, "GET /app.js HTTP/1.1\r\n", 1000, 0);
    assert(client_step(&partial, true, false, 0, answer) == CLIENT_WAITING);
    assert(client_step(&partial, false, false, CLIENT_TIMEOUT_US + 1, answer) == CLIENT_DROPPED);
    puts("web_client_test ok");
    return 0;
}
