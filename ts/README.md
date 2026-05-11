# @jsonic/zon

A [Jsonic](https://jsonic.senecajs.org) syntax plugin that parses
[Zig Object Notation (ZON)](https://ziglang.org/documentation/master/#ZON)
text into objects, arrays, and scalar values. Available for
TypeScript and Go.

ZON is the data format used for Zig `build.zig.zon` manifests and
similar configuration files. It is based on Zig anonymous struct
literals, and looks like this:

```zon
.{
    .name = "example",
    .version = "0.0.1",
    .dependencies = .{
        .foo = .{
            .url = "https://example.com/foo.tar.gz",
            .hash = "1220deadbeef",
        },
    },
    .paths = .{
        "build.zig",
        "src",
    },
}
```

## Quick example

**TypeScript**

```typescript
import { Jsonic } from 'jsonic'
import { Zon } from '@jsonic/zon'

const parse = Jsonic.make().use(Zon)

parse('.{ .name = "Alice", .age = 30 }')
// { name: 'Alice', age: 30 }

parse('.{ 1, 2, 3 }')
// [1, 2, 3]
```

**Go**

```go
import zon "github.com/jsonicjs/zon/go"

result, _ := zon.Parse(`.{ .name = "Alice", .age = 30 }`)
// map[string]any{"name": "Alice", "age": 30}
```

## Documentation

Full documentation following the [Diataxis](https://diataxis.fr)
framework (tutorials, how-to guides, explanation, reference):

- [TypeScript documentation](doc/zon-ts.md)
- [Go documentation](doc/zon-go.md)

## License

Copyright (c) 2025 Richard Rodger and other contributors,
[MIT License](LICENSE).
