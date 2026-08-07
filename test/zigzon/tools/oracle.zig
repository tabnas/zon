// ZON oracle — the ONLY authority in this repo on what a ZON document means.
//
// Reads length-prefixed documents on stdin, emits one JSON line per document
// describing how the REFERENCE Zig implementation classifies it:
// `std.zig.Ast` in `.zon` mode (syntax) followed by `std.zig.ZonGen` (lowering).
//
//   {"ok":true,"value":<canonical JSON>}
//   {"ok":false,"error":"<first message>"}
//
// ZonGen is the correct comparison layer. It lowers ZON to an UNTYPED value
// tree, which is exactly what @tabnas/zon produces. The layer above it,
// `std.zon.parse`, additionally type-checks the tree against a Zig type; those
// failures are type errors, not ZON errors, and must not be counted as such.
//
// Canonical JSON encoding of a Zoir node:
//   true / false / null       -> true / false / null
//   inf / -inf / nan          -> "@inf" / "@-inf" / "@nan"
//   int literal               -> JSON number when exactly representable as f64,
//                                otherwise {"$big":"<decimal>"}
//   float literal             -> JSON number (f64), or "@inf" / "@nan"
//   char literal 'x'          -> JSON number (the codepoint)
//   enum literal .foo         -> {"$enum":"foo"}
//   string literal            -> JSON string
//   .{}                       -> []
//   array literal             -> JSON array
//   struct literal            -> JSON object
//
// Pinned to zig 0.16.0 (see scripts/fetch-zigzon.sh); the fetch script refuses
// to run against any other version.
const std = @import("std");
const Ast = std.zig.Ast;
const Zoir = std.zig.Zoir;
const ZonGen = std.zig.ZonGen;

var out: std.ArrayList(u8) = .empty;
var gpa_alloc: std.mem.Allocator = undefined;

fn w(bytes: []const u8) void {
    out.appendSlice(gpa_alloc, bytes) catch @panic("oom");
}

fn wstr(s: []const u8) void {
    w("\"");
    for (s) |c| {
        switch (c) {
            '"' => w("\\\""),
            '\\' => w("\\\\"),
            '\n' => w("\\n"),
            '\r' => w("\\r"),
            '\t' => w("\\t"),
            else => {
                if (c < 0x20) {
                    var buf: [8]u8 = undefined;
                    const s2 = std.fmt.bufPrint(&buf, "\\u{x:0>4}", .{c}) catch unreachable;
                    w(s2);
                } else {
                    out.append(gpa_alloc, c) catch @panic("oom");
                }
            },
        }
    }
    w("\"");
}

fn wfmt(comptime fmt: []const u8, args: anytype) void {
    var buf: [4096]u8 = undefined;
    const s = std.fmt.bufPrint(&buf, fmt, args) catch @panic("fmt");
    w(s);
}

// Emit an f64 in a form JSON.parse round-trips to the same double.
fn wf64(v: f64) void {
    if (std.math.isNan(v)) return wstr("@nan");
    if (std.math.isInf(v)) return wstr(if (v > 0) "@inf" else "@-inf");
    // Negative zero must survive: `-0.0` is a distinct ZON value, and the
    // integer path below would silently render it as `0`, wrongly failing a
    // parser that got it right (and wrongly passing one that did not).
    if (v == 0 and std.math.signbit(v)) return w("-0");
    if (v == @trunc(v) and @abs(v) < 1e15) {
        wfmt("{d}", .{@as(i64, @intFromFloat(v))});
    } else {
        wfmt("{d}", .{v});
    }
}

fn emit(zoir: Zoir, idx: Zoir.Node.Index) void {
    switch (idx.get(zoir)) {
        .true => w("true"),
        .false => w("false"),
        .null => w("null"),
        .pos_inf => wstr("@inf"),
        .neg_inf => wstr("@-inf"),
        .nan => wstr("@nan"),
        .int_literal => |int| switch (int) {
            .small => |n| wfmt("{d}", .{n}),
            .big => |b| {
                // Exactly representable as f64?
                const tf = b.toFloat(f64, .nearest_even);
                const as_f: f64 = tf[0];
                const ok = std.math.isFinite(as_f) and tf[1] == .exact;
                if (ok) {
                    wf64(as_f);
                } else {
                    w("{\"$big\":");
                    const s = b.toStringAlloc(gpa_alloc, 10, .lower) catch @panic("oom");
                    defer gpa_alloc.free(s);
                    wstr(s);
                    w("}");
                }
            },
        },
        .float_literal => |f| wf64(@floatCast(f)),
        .char_literal => |c| wfmt("{d}", .{c}),
        .enum_literal => |e| {
            w("{\"$enum\":");
            wstr(e.get(zoir));
            w("}");
        },
        .string_literal => |s| wstr(s),
        .empty_literal => w("[]"),
        .array_literal => |r| {
            w("[");
            for (0..r.len) |i| {
                if (i > 0) w(",");
                emit(zoir, r.at(@intCast(i)));
            }
            w("]");
        },
        .struct_literal => |sl| {
            w("{");
            for (sl.names, 0..) |name, i| {
                if (i > 0) w(",");
                wstr(name.get(zoir));
                w(":");
                emit(zoir, sl.vals.at(@intCast(i)));
            }
            w("}");
        },
    }
}

pub fn main(init: std.process.Init) !void {
    const io = init.io;
    var arena_inst = std.heap.ArenaAllocator.init(std.heap.page_allocator);
    defer arena_inst.deinit();
    const gpa = arena_inst.allocator();
    gpa_alloc = gpa;

    var stdout_buffer: [64 * 1024]u8 = undefined;
    var stdout_writer = std.Io.File.stdout().writer(io, &stdout_buffer);
    const stdout = &stdout_writer.interface;

    // Read all of stdin. std.posix.read is the stable low-level surface across
    // the releases where std.Io itself keeps moving.
    var in_buf: std.ArrayList(u8) = .empty;
    {
        var chunk: [64 * 1024]u8 = undefined;
        while (true) {
            const n = try std.posix.read(0, &chunk);
            if (n == 0) break;
            try in_buf.appendSlice(gpa, chunk[0..n]);
        }
    }
    const input = in_buf.items;

    var pos: usize = 0;
    while (pos < input.len) {
        // "<decimal length>\n<bytes>"
        const nl = std.mem.indexOfScalarPos(u8, input, pos, '\n') orelse break;
        const len = try std.fmt.parseInt(usize, input[pos..nl], 10);
        const src = input[nl + 1 ..][0..len];
        pos = nl + 1 + len;

        const srcz = try gpa.dupeZ(u8, src);

        out.clearRetainingCapacity();
        var tree = try Ast.parse(gpa, srcz, .zon);
        defer tree.deinit(gpa);

        if (tree.errors.len > 0) {
            w("{\"ok\":false,\"error\":");
            var buf: [512]u8 = undefined;
            var aw: std.Io.Writer = .fixed(&buf);
            tree.renderError(tree.errors[0], &aw) catch {};
            wstr(aw.buffered());
            w("}");
        } else {
            var zoir = try ZonGen.generate(gpa, tree, .{});
            defer zoir.deinit(gpa);
            if (zoir.hasCompileErrors()) {
                w("{\"ok\":false,\"error\":");
                wstr(zoir.compile_errors[0].msg.get(zoir));
                w("}");
            } else {
                w("{\"ok\":true,\"value\":");
                emit(zoir, .root);
                w("}");
            }
        }
        w("\n");
        try stdout.writeAll(out.items);
    }
    try stdout.flush();
}
