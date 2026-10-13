/* SPDX-License-Identifier: MIT
 * A stand-in libretro core for testing reborn-retro without an emulator
 * or a game. Its "game" is a text file: "<width> <height> <pixel format>
 * <sample rate> [quarter turns to ask for]". It draws a test card (an 8-pixel checker, a white
 * border, colour bars, and a box that lights up while A is held), plays a
 * square wave, counts presses of A in its battery memory and logs every
 * change of the buttons to stderr as "fake: input <mask>". */
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef bool (*env_t)(unsigned, void *);
typedef void (*video_t)(const void *, unsigned, unsigned, size_t);
typedef void (*sample_t)(int16_t, int16_t);
typedef size_t (*batch_t)(const int16_t *, size_t);
typedef void (*poll_t)(void);
typedef int16_t (*state_t)(unsigned, unsigned, unsigned, unsigned);

struct sys_info { const char *name, *version, *ext; bool need_fullpath, block_extract; };
struct geometry { unsigned bw, bh, mw, mh; float aspect; };
struct timing { double fps, rate; };
struct av_info { struct geometry g; struct timing t; };
struct game_info { const char *path; const void *data; size_t size; const char *meta; };
struct variable { const char *key, *value; };

static env_t env;
static video_t video;
static batch_t batch;
static poll_t poll_cb;
static state_t input;
static unsigned w = 256, h = 240, fmt = 2, rate = 48000;
static uint8_t *fb;
static uint8_t sram[64];
static uint32_t frames, phase;
static int last_mask = -1;
static int a_was_down;

void retro_set_environment(env_t e) {
  env = e;
  static const struct variable vars[] = {
      {"fake_colour", "Bar colour; blue|green"}, {NULL, NULL}};
  env(16, (void *)vars);
}
void retro_set_video_refresh(video_t v) { video = v; }
void retro_set_audio_sample(sample_t s) { (void)s; }
void retro_set_audio_sample_batch(batch_t b) { batch = b; }
void retro_set_input_poll(poll_t p) { poll_cb = p; }
void retro_set_input_state(state_t s) { input = s; }
void retro_init(void) {}
void retro_deinit(void) { free(fb); fb = NULL; }
unsigned retro_api_version(void) { return 1; }
void retro_get_system_info(struct sys_info *i) {
  memset(i, 0, sizeof(*i));
  i->name = "fake";
  i->version = "1";
  i->ext = "fake";
}
void retro_get_system_av_info(struct av_info *i) {
  i->g.bw = i->g.mw = w;
  i->g.bh = i->g.mh = h;
  i->g.aspect = 0;
  i->t.fps = 60.0;
  i->t.rate = rate;
}
void retro_set_controller_port_device(unsigned p, unsigned d) { (void)p; (void)d; }
void retro_reset(void) {}

static unsigned bpp(void) { return fmt == 1 ? 4 : 2; }
static void put(unsigned x, unsigned y, uint8_t r, uint8_t g, uint8_t b) {
  uint8_t *p = fb + (y * (w + 16) + x) * bpp(); /* rows padded: a pitch */
  if (fmt == 1) {
    uint32_t v = (uint32_t)r << 16 | (uint32_t)g << 8 | b;
    memcpy(p, &v, 4);
  } else if (fmt == 2) {
    uint16_t v = (uint16_t)((r >> 3) << 11 | (g >> 2) << 5 | (b >> 3));
    memcpy(p, &v, 2);
  } else {
    uint16_t v = (uint16_t)((r >> 3) << 10 | (g >> 3) << 5 | (b >> 3));
    memcpy(p, &v, 2);
  }
}
static void draw(int mask) {
  struct variable v = {"fake_colour", NULL};
  int green = env(15, &v) && v.value && !strcmp(v.value, "green");
  static const uint8_t bars[6][3] = {{255, 255, 255}, {255, 220, 0}, {0, 220, 220},
                                     {0, 200, 0},     {220, 0, 220}, {220, 0, 0}};
  for (unsigned y = 0; y < h; y++)
    for (unsigned x = 0; x < w; x++) {
      uint8_t c = ((x / 8 + y / 8) & 1) ? 90 : 50;
      uint8_t r = c, g = c, b = c;
      if (x == 0 || y == 0 || x == w - 1 || y == h - 1)
        r = g = b = 255;
      else if (y >= h / 8 && y < h / 4 && x >= 8 && x < w - 8) {
        unsigned i = (x - 8) * 6 / (w - 16);
        r = bars[i][0], g = bars[i][1], b = bars[i][2];
      } else if (y >= h - h / 4 && y < h - h / 8 && x >= w / 4 && x < w - w / 4) {
        r = 0, g = green ? 160 : 0, b = green ? 0 : 200; /* the option's colour */
      }
      unsigned bx = w / 2 - 12, by = h / 2 - 12;
      if (x >= bx && x < bx + 24 && y >= by && y < by + 24) {
        int on = mask & (1 << 8); /* A */
        r = on ? 255 : 120, g = on ? 40 : 120, b = on ? 40 : 120;
      }
      put(x, y, r, g, b);
    }
}

bool retro_load_game(const struct game_info *g) {
  if (!g || !g->data || g->size == 0)
    return false;
  char text[64] = {0};
  memcpy(text, g->data, g->size < 63 ? g->size : 63);
  if (!strncmp(text, "fail", 4))
    return false;
  unsigned rotation = 0;
  sscanf(text, "%u %u %u %u %u", &w, &h, &fmt, &rate, &rotation);
  if (!env(10, &fmt))
    return false;
  /* Like FBNeo with a vertical game: draw on the side, ask for a turn. */
  if (rotation && !env(1, &rotation))
    return false;
  fb = calloc((size_t)(w + 16) * h, 4);
  return fb != NULL;
}
void retro_unload_game(void) {}

void retro_run(void) {
  poll_cb();
  int mask = input(0, 1, 0, 256);
  int a = input(0, 1, 0, 8) != 0;
  if (mask != last_mask) {
    fprintf(stderr, "fake: input %d\n", mask);
    draw(mask);
    last_mask = mask;
  }
  if (a && !a_was_down)
    sram[0]++;
  a_was_down = a;
  frames++;
  video(fb, w, h, (size_t)(w + 16) * bpp());
  /* A 500 Hz square wave, one frame's worth. */
  static int16_t buf[2 * 4000];
  unsigned n = rate / 60;
  if (n > 4000)
    n = 4000;
  for (unsigned i = 0; i < n; i++, phase++) {
    int16_t s = (phase * 1000 / rate) & 1 ? 3000 : -3000;
    buf[2 * i] = buf[2 * i + 1] = s;
  }
  batch(buf, n);
}

size_t retro_serialize_size(void) { return 4 + sizeof(sram); }
bool retro_serialize(void *d, size_t n) {
  if (n < retro_serialize_size())
    return false;
  memcpy(d, &frames, 4);
  memcpy((uint8_t *)d + 4, sram, sizeof(sram));
  return true;
}
bool retro_unserialize(const void *d, size_t n) {
  if (n != retro_serialize_size())
    return false;
  memcpy(&frames, d, 4);
  memcpy(sram, (const uint8_t *)d + 4, sizeof(sram));
  return true;
}
void *retro_get_memory_data(unsigned id) { return id == 0 ? sram : NULL; }
size_t retro_get_memory_size(unsigned id) { return id == 0 ? sizeof(sram) : 0; }
