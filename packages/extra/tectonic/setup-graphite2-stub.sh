#!/bin/sh
# setup-graphite2-stub.sh — build a no-op libgraphite2 shim so Tectonic
# (XeTeX engine + vendored harfbuzz with HB_HAVE_GRAPHITE2) links
# WITHOUT pulling the real LGPL-2.1 libgraphite2 into the binary.
#
# The shim provides:
#   1. A no-op libgraphite2.a + libgraphite2.so under $STUB_DIR/lib/
#      — every gr_* C function returns NULL / 0 / a no-op.
#   2. Public headers under $STUB_DIR/include/graphite2/{Font,Segment,Types}.h
#      mirroring the upstream API surface that harfbuzz's hb-graphite2.cc
#      and tectonic's xetex-ext.c compile against.
#   3. A graphite2.pc file under $STUB_DIR/lib/pkgconfig/ so Tectonic's
#      bridge_graphite2/build.rs pkg-config probe resolves cleanly.
#
# Behaviour at runtime: every call into the stub library is a no-op.
# harfbuzz's gr_face() accessor sees gr_make_face_with_ops() return NULL
# and reports "no graphite2 face" to the Rust shaper, which short-circuits
# Graphite shaping in xetex_layout/src/engine.rs.  OpenType shaping
# (the 99.9% case) works normally.  Users who request `Renderer=Graphite`
# in their .tex file get the same "Graphite shaper unavailable" path
# they'd see if harfbuzz had been built without HB_HAVE_GRAPHITE2.
#
# Usage:
#   STUB_DIR=/tmp/graphite2-stub setup-graphite2-stub.sh
#   export PKG_CONFIG_PATH="$STUB_DIR/lib/pkgconfig:$PKG_CONFIG_PATH"
#
# License of THIS file: 0BSD (matches jonerix's recipe-helper convention).
set -e

: "${STUB_DIR:?STUB_DIR must be set}"
: "${CC:=clang}"
: "${AR:=llvm-ar}"

mkdir -p "$STUB_DIR/include/graphite2" "$STUB_DIR/lib/pkgconfig"

# -- 1. Stub headers ---------------------------------------------------
#
# We collapse Types.h, Font.h, and Segment.h into self-contained files
# rather than chasing the upstream layout exactly.  Each header declares
# every symbol that harfbuzz 11.x + Tectonic 0.16.x reference, with
# opaque struct types and matching ABI signatures.
#
# The symbol set was derived by grep'ing for `gr_*` references in:
#   - tectonic/crates/bridge_graphite2/src/sys.rs   (Rust extern decls)
#   - tectonic/crates/engine_xetex/xetex/xetex-ext.c (C call sites)
#   - harfbuzz/src/hb-graphite2.cc                  (vendored under
#                                                    crates/bridge_harfbuzz/
#                                                    harfbuzz/)
# If a future Tectonic or harfbuzz release adds new gr_* references, the
# build will fail at link time with an undefined-symbol error pointing
# at the missing function — add a no-op for it here and rebuild.

cat > "$STUB_DIR/include/graphite2/Types.h" <<'HEADER'
#ifndef GRAPHITE2_TYPES_H
#define GRAPHITE2_TYPES_H
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef uint32_t gr_uint32;
typedef uint16_t gr_uint16;
typedef int32_t  gr_int32;
typedef int16_t  gr_int16;
typedef uint8_t  gr_uint8;
typedef int8_t   gr_int8;

typedef struct gr_face        gr_face;
typedef struct gr_feature_ref gr_feature_ref;
typedef struct gr_feature_val gr_feature_val;
typedef struct gr_font        gr_font;
typedef struct gr_segment     gr_segment;
typedef struct gr_slot        gr_slot;
typedef struct gr_char_info   gr_char_info;

enum gr_encform { gr_utf8 = 1, gr_utf16 = 2, gr_utf32 = 4 };
typedef enum gr_encform gr_encform;

enum gr_break_weight {
    gr_breakNone        = 0,
    gr_breakWhitespace  = 10,
    gr_breakWord        = 15,
    gr_breakIntra       = 20,
    gr_breakLetter      = 30,
    gr_breakClip        = 40,
    gr_breakBeforeWhitespace = -10,
    gr_breakBeforeWord        = -15,
    gr_breakBeforeIntra       = -20,
    gr_breakBeforeLetter      = -30,
    gr_breakBeforeClip        = -40
};
typedef enum gr_break_weight gr_break_weight;

enum gr_face_options {
    gr_face_default       = 0,
    gr_face_dumbRendering = 1,
    gr_face_cacheCmap     = 2,
    gr_face_preloadGlyphs = 4,
    gr_face_preloadAll    = (gr_face_preloadGlyphs | gr_face_cacheCmap)
};
typedef enum gr_face_options gr_face_options;

typedef const void* (*gr_get_table_fn)(const void* appFaceHandle, gr_uint32 name, size_t* len);

typedef struct {
    size_t            size;
    gr_get_table_fn   get_table;
    void              *release_table;
} gr_face_ops;

#ifdef __cplusplus
}
#endif
#endif
HEADER

cat > "$STUB_DIR/include/graphite2/Font.h" <<'HEADER'
#ifndef GRAPHITE2_FONT_H
#define GRAPHITE2_FONT_H
#include <graphite2/Types.h>

#ifdef __cplusplus
extern "C" {
#endif

gr_face* gr_make_face(const void* appFaceHandle, gr_get_table_fn getTable, unsigned int faceOptions);
gr_face* gr_make_face_with_ops(const void* appFaceHandle, const gr_face_ops* ops, unsigned int faceOptions);
gr_face* gr_make_face_with_seg_callback(const void* appFaceHandle, gr_get_table_fn getTable, void* segCacheCallback, unsigned int faceOptions);
void     gr_face_destroy(gr_face* face);

gr_uint16            gr_face_n_fref(const gr_face* pFace);
const gr_feature_ref* gr_face_fref(const gr_face* pFace, gr_uint16 i);
const gr_feature_ref* gr_face_find_fref(const gr_face* pFace, gr_uint32 featId);
gr_feature_val*      gr_face_featureval_for_lang(const gr_face* pFace, gr_uint32 langname);

gr_uint32            gr_fref_id(const gr_feature_ref* pfeatureref);
gr_uint16            gr_fref_n_values(const gr_feature_ref* pfeatureref);
gr_int16             gr_fref_value(const gr_feature_ref* pfeatureref, gr_uint16 settingno);
gr_uint16            gr_fref_feature_value(const gr_feature_ref* pfeatureref, const gr_feature_val* feats);
int                  gr_fref_set_feature_value(const gr_feature_ref* pfeatureref, gr_uint16 val, gr_feature_val* pDest);
void*                gr_fref_label(const gr_feature_ref* pfeatureref, gr_uint16* langId, gr_encform utf, gr_uint32* length);
void*                gr_fref_value_label(const gr_feature_ref* pfeatureref, gr_uint16 setting, gr_uint16* lang_id, gr_encform utf, gr_uint32* length);
void                 gr_label_destroy(void* label);
void                 gr_featureval_destroy(gr_feature_val* p);

gr_font* gr_make_font(float ppm, const gr_face* face);
void     gr_font_destroy(gr_font* p);

#ifdef __cplusplus
}
#endif
#endif
HEADER

cat > "$STUB_DIR/include/graphite2/Segment.h" <<'HEADER'
#ifndef GRAPHITE2_SEGMENT_H
#define GRAPHITE2_SEGMENT_H
#include <graphite2/Types.h>
#include <graphite2/Font.h>

#ifdef __cplusplus
extern "C" {
#endif

gr_segment* gr_make_seg(const gr_font* font, const gr_face* face, gr_uint32 script,
                        const gr_feature_val* pFeats, gr_encform enc,
                        const void* pStart, size_t nChars, int dir);
void        gr_seg_destroy(gr_segment* p);
float       gr_seg_advance_X(const gr_segment* pSeg);
float       gr_seg_advance_Y(const gr_segment* pSeg);
unsigned int gr_seg_n_cinfo(const gr_segment* pSeg);
const gr_char_info* gr_seg_cinfo(const gr_segment* pSeg, unsigned int index);
unsigned int gr_seg_n_slots(const gr_segment* pSeg);
const gr_slot* gr_seg_first_slot(gr_segment* pSeg);
const gr_slot* gr_seg_last_slot(gr_segment* pSeg);

const gr_slot* gr_slot_next_in_segment(const gr_slot* p);
const gr_slot* gr_slot_prev_in_segment(const gr_slot* p);
const gr_slot* gr_slot_attached_to(const gr_slot* p);
const gr_slot* gr_slot_first_attachment(const gr_slot* p);
const gr_slot* gr_slot_next_sibling_attachment(const gr_slot* p);
unsigned short gr_slot_gid(const gr_slot* p);
float          gr_slot_origin_X(const gr_slot* p);
float          gr_slot_origin_Y(const gr_slot* p);
float          gr_slot_advance_X(const gr_slot* p, const gr_face* face, const gr_font* font);
float          gr_slot_advance_Y(const gr_slot* p, const gr_face* face, const gr_font* font);
int            gr_slot_before(const gr_slot* p);
int            gr_slot_after(const gr_slot* p);
unsigned int   gr_slot_index(const gr_slot* p);
int            gr_slot_can_insert_before(const gr_slot* p);
int            gr_slot_original(const gr_slot* p);
int            gr_slot_attr(const gr_slot* p, const gr_segment* pSeg, int index, gr_uint8 subindex);

unsigned int       gr_cinfo_unicode_char(const gr_char_info* p);
int                gr_cinfo_break_weight(const gr_char_info* p);
int                gr_cinfo_after(const gr_char_info* p);
int                gr_cinfo_before(const gr_char_info* p);
size_t             gr_cinfo_base(const gr_char_info* p);

#ifdef __cplusplus
}
#endif
#endif
HEADER

# -- 2. Stub C library ------------------------------------------------
#
# Every function returns the most-benign value its prototype allows.
# Pointer returners return NULL.  Numeric returners return 0.  Void
# functions are no-ops.  Float returners return 0.0f.
#
# A NULL gr_face* propagates through harfbuzz's setup_graphite2_face
# code path as "no Graphite support" — exactly what we want.

cat > "$STUB_DIR/graphite2-stub.c" <<'STUB'
#include <stddef.h>
#include <stdint.h>

/* Opaque types — declared in headers, never dereferenced here. */
typedef struct gr_face        gr_face;
typedef struct gr_feature_ref gr_feature_ref;
typedef struct gr_feature_val gr_feature_val;
typedef struct gr_font        gr_font;
typedef struct gr_segment     gr_segment;
typedef struct gr_slot        gr_slot;
typedef struct gr_char_info   gr_char_info;

/* -- Face --------------------------------------------------------- */
gr_face* gr_make_face                    (const void* h, void* g, unsigned int o)                    { (void)h;(void)g;(void)o; return NULL; }
gr_face* gr_make_face_with_ops           (const void* h, const void* ops, unsigned int o)           { (void)h;(void)ops;(void)o; return NULL; }
gr_face* gr_make_face_with_seg_callback  (const void* h, void* g, void* s, unsigned int o)          { (void)h;(void)g;(void)s;(void)o; return NULL; }
void     gr_face_destroy                 (gr_face* f)                                                { (void)f; }
uint16_t gr_face_n_fref                  (const gr_face* f)                                          { (void)f; return 0; }
const gr_feature_ref* gr_face_fref       (const gr_face* f, uint16_t i)                              { (void)f;(void)i; return NULL; }
const gr_feature_ref* gr_face_find_fref  (const gr_face* f, uint32_t id)                             { (void)f;(void)id; return NULL; }
gr_feature_val* gr_face_featureval_for_lang(const gr_face* f, uint32_t lang)                        { (void)f;(void)lang; return NULL; }

/* -- Feature refs ------------------------------------------------- */
uint32_t gr_fref_id                      (const gr_feature_ref* r)                                   { (void)r; return 0; }
uint16_t gr_fref_n_values                (const gr_feature_ref* r)                                   { (void)r; return 0; }
int16_t  gr_fref_value                   (const gr_feature_ref* r, uint16_t s)                       { (void)r;(void)s; return 0; }
uint16_t gr_fref_feature_value           (const gr_feature_ref* r, const gr_feature_val* v)          { (void)r;(void)v; return 0; }
int      gr_fref_set_feature_value       (const gr_feature_ref* r, uint16_t v, gr_feature_val* d)    { (void)r;(void)v;(void)d; return 0; }
void*    gr_fref_label                   (const gr_feature_ref* r, uint16_t* l, int e, uint32_t* n)  { (void)r;(void)l;(void)e;(void)n; return NULL; }
void*    gr_fref_value_label             (const gr_feature_ref* r, uint16_t s, uint16_t* l, int e, uint32_t* n) { (void)r;(void)s;(void)l;(void)e;(void)n; return NULL; }
void     gr_label_destroy                (void* l)                                                   { (void)l; }
void     gr_featureval_destroy           (gr_feature_val* p)                                         { (void)p; }

/* -- Font --------------------------------------------------------- */
gr_font* gr_make_font                    (float ppm, const gr_face* f)                               { (void)ppm;(void)f; return NULL; }
void     gr_font_destroy                 (gr_font* p)                                                { (void)p; }

/* -- Segment ------------------------------------------------------ */
gr_segment* gr_make_seg                  (const gr_font* fo, const gr_face* fa, uint32_t s, const gr_feature_val* fv, int enc, const void* st, size_t n, int dir) {
    (void)fo;(void)fa;(void)s;(void)fv;(void)enc;(void)st;(void)n;(void)dir; return NULL;
}
void  gr_seg_destroy                     (gr_segment* p)                                             { (void)p; }
float gr_seg_advance_X                   (const gr_segment* p)                                       { (void)p; return 0.0f; }
float gr_seg_advance_Y                   (const gr_segment* p)                                       { (void)p; return 0.0f; }
unsigned int gr_seg_n_cinfo              (const gr_segment* p)                                       { (void)p; return 0; }
const gr_char_info* gr_seg_cinfo         (const gr_segment* p, unsigned int i)                       { (void)p;(void)i; return NULL; }
unsigned int gr_seg_n_slots              (const gr_segment* p)                                       { (void)p; return 0; }
const gr_slot* gr_seg_first_slot         (gr_segment* p)                                             { (void)p; return NULL; }
const gr_slot* gr_seg_last_slot          (gr_segment* p)                                             { (void)p; return NULL; }

/* -- Slot --------------------------------------------------------- */
const gr_slot* gr_slot_next_in_segment   (const gr_slot* p)                                          { (void)p; return NULL; }
const gr_slot* gr_slot_prev_in_segment   (const gr_slot* p)                                          { (void)p; return NULL; }
const gr_slot* gr_slot_attached_to       (const gr_slot* p)                                          { (void)p; return NULL; }
const gr_slot* gr_slot_first_attachment  (const gr_slot* p)                                          { (void)p; return NULL; }
const gr_slot* gr_slot_next_sibling_attachment(const gr_slot* p)                                     { (void)p; return NULL; }
unsigned short gr_slot_gid               (const gr_slot* p)                                          { (void)p; return 0; }
float          gr_slot_origin_X          (const gr_slot* p)                                          { (void)p; return 0.0f; }
float          gr_slot_origin_Y          (const gr_slot* p)                                          { (void)p; return 0.0f; }
float          gr_slot_advance_X         (const gr_slot* p, const gr_face* f, const gr_font* fo)     { (void)p;(void)f;(void)fo; return 0.0f; }
float          gr_slot_advance_Y         (const gr_slot* p, const gr_face* f, const gr_font* fo)     { (void)p;(void)f;(void)fo; return 0.0f; }
int            gr_slot_before            (const gr_slot* p)                                          { (void)p; return 0; }
int            gr_slot_after             (const gr_slot* p)                                          { (void)p; return 0; }
unsigned int   gr_slot_index             (const gr_slot* p)                                          { (void)p; return 0; }
int            gr_slot_can_insert_before (const gr_slot* p)                                          { (void)p; return 0; }
int            gr_slot_original          (const gr_slot* p)                                          { (void)p; return 0; }
int            gr_slot_attr              (const gr_slot* p, const gr_segment* s, int i, uint8_t sub) { (void)p;(void)s;(void)i;(void)sub; return 0; }

/* -- char info ---------------------------------------------------- */
unsigned int gr_cinfo_unicode_char       (const gr_char_info* p)                                     { (void)p; return 0; }
int          gr_cinfo_break_weight       (const gr_char_info* p)                                     { (void)p; return 0; }
int          gr_cinfo_after              (const gr_char_info* p)                                     { (void)p; return 0; }
int          gr_cinfo_before             (const gr_char_info* p)                                     { (void)p; return 0; }
size_t       gr_cinfo_base               (const gr_char_info* p)                                     { (void)p; return 0; }
STUB

"$CC" -c -O2 -fPIC -I"$STUB_DIR/include" \
      -o "$STUB_DIR/graphite2-stub.o" "$STUB_DIR/graphite2-stub.c"
"$AR" rcs "$STUB_DIR/lib/libgraphite2.a" "$STUB_DIR/graphite2-stub.o"

# Shared lib — Linux uses -soname, macOS uses -install_name.  The
# script is meant for the jonerix Linux builder, but the macOS path
# keeps local dev sanity-checks working.
case "$(uname -s)" in
    Darwin)
        "$CC" -dynamiclib -fPIC \
              -Wl,-install_name,libgraphite2.so.3 \
              -o "$STUB_DIR/lib/libgraphite2.so.3.2.1" \
              "$STUB_DIR/graphite2-stub.o"
        ;;
    *)
        "$CC" -shared -fPIC \
              -Wl,-soname,libgraphite2.so.3 \
              -o "$STUB_DIR/lib/libgraphite2.so.3.2.1" \
              "$STUB_DIR/graphite2-stub.o"
        ;;
esac
ln -sf libgraphite2.so.3.2.1 "$STUB_DIR/lib/libgraphite2.so.3"
ln -sf libgraphite2.so.3     "$STUB_DIR/lib/libgraphite2.so"

# -- 3. pkg-config metadata -------------------------------------------
cat > "$STUB_DIR/lib/pkgconfig/graphite2.pc" <<PKG
prefix=$STUB_DIR
exec_prefix=\${prefix}
libdir=\${exec_prefix}/lib
includedir=\${prefix}/include

Name: graphite2
Description: Graphite2 LGPL-free stub for jonerix tectonic (no-op shaper)
Version: 1.3.14
Libs: -L\${libdir} -lgraphite2
Cflags: -I\${includedir}
PKG

echo "graphite2-stub: installed under $STUB_DIR" >&2
