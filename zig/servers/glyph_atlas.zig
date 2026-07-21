// Glyph atlas: 512x512 BGRA8 texture with shelf-packing.
// Caches rendered bitmap font glyphs; returns UV coordinates for each.

const font = @import("bitmap_font.zig");

pub const ATLAS_W: u32 = 512;
pub const ATLAS_H: u32 = 512;

pub const GlyphInfo = struct {
    u0: u32, // left pixel column in atlas
    v0: u32, // top pixel row in atlas
    w: u32,
    h: u32,
};

// Atlas pixel buffer — BGRA8, 512×512 = 1 MB
pub var atlas: [ATLAS_W * ATLAS_H]u32 = [_]u32{0} ** (ATLAS_W * ATLAS_H);

// Shelf-packer state
var shelf_x: u32 = 0;
var shelf_y: u32 = 0;
var shelf_h: u32 = 0;

// Cache: one entry per printable ASCII character (0x20..0x7E)
const CACHE_SIZE: usize = 95;
var cache: [CACHE_SIZE]?GlyphInfo = [_]?GlyphInfo{null} ** CACHE_SIZE;

// Rasterize a glyph into the atlas and return its location.
fn rasterize(ch: u8, fg: u32) GlyphInfo {
    const gw: u32 = font.GLYPH_W;
    const gh: u32 = font.GLYPH_H;

    // Shelf-pack: if this glyph doesn't fit on the current shelf, start a new one
    if (shelf_x + gw > ATLAS_W) {
        shelf_y += shelf_h;
        shelf_x = 0;
        shelf_h = 0;
    }
    if (shelf_h < gh) shelf_h = gh;

    const bmp = font.getGlyph(ch);
    const dst_x = shelf_x;
    const dst_y = shelf_y;

    for (0..gh) |row| {
        const bits = bmp[row];
        for (0..gw) |col| {
            const pixel: u32 = if ((bits >> @intCast(7 - col)) & 1 != 0) fg else 0x00000000;
            atlas[(dst_y + row) * ATLAS_W + (dst_x + col)] = pixel;
        }
    }

    shelf_x += gw;

    return .{ .u0 = dst_x, .v0 = dst_y, .w = gw, .h = gh };
}

// Look up or rasterize a glyph; returns its atlas coordinates.
pub fn getGlyph(ch: u8, fg: u32) GlyphInfo {
    const c: u8 = if (ch >= 0x20 and ch <= 0x7E) ch else '?';
    const idx = c - 0x20;
    if (cache[idx]) |info| return info;
    const info = rasterize(c, fg);
    cache[idx] = info;
    return info;
}

// Invalidate the cache (e.g. if fg colour changes globally)
pub fn reset() void {
    shelf_x = 0;
    shelf_y = 0;
    shelf_h = 0;
    for (&cache) |*e| e.* = null;
    for (&atlas) |*p| p.* = 0;
}
