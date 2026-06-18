# @tabnas/zon

A [Tabnas](https://github.com/tabnas/parser) syntax plugin that parses
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

```js
import { Tabnas } from '@tabnas/parser'
import { jsonic } from '@tabnas/jsonic'
import { Zon } from '@tabnas/zon'

const parse = new Tabnas().use(jsonic).use(Zon)

parse.parse('.{ .name = "Alice", .age = 30 }') // => { name: 'Alice', age: 30 }

parse.parse('.{ 1, 2, 3 }') // => [1, 2, 3]
```

**Go**

```go
import zon "github.com/tabnas/zon/go"

result, _ := zon.Parse(`.{ .name = "Alice", .age = 30 }`)
// map[string]any{"name": "Alice", "age": 30}
```

## Documentation

Full documentation following the [Diataxis](https://diataxis.fr)
framework (tutorials, how-to guides, explanation, reference):

- [TypeScript documentation](doc/zon-ts.md)
- [Go documentation](doc/zon-go.md)


## Grammar diagram

The grammar is defined in the top-level [`zon-grammar.jsonic`](../zon-grammar.jsonic)
and embedded into this implementation (and the Go port) by
[`embed-grammar.js`](embed-grammar.js) during the build.

The installed grammar as a railroad/syntax diagram, generated from the live
grammar with [`@tabnas/railroad`](https://github.com/tabnas/railroad):

![zon grammar railroad diagram](doc/grammar.svg)

A vertical ASCII version is in [`doc/grammar.txt`](doc/grammar.txt).

## License

Copyright (c) 2025 Richard Rodger and other contributors,
[MIT License](LICENSE).
