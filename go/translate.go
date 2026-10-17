package tabnaszon

import _ "embed"

// TranslationPart is one optional alchemy source and the entry point a host calls.
type TranslationPart struct {
	Entry  string
	Source string
}

// TranslationParts is the package-local structural translation interface.
type TranslationParts struct {
	Manifest string
	Lift     *TranslationPart
	Embed    *TranslationPart
	Render   *TranslationPart
	// Transduce is the transducer: the format's JSON events from its
	// parser's rule passes, for a host that reads parser events.
	Transduce *TranslationPart
}

//go:embed translate/manifest.json
var translationManifest string

//go:embed translate/render.alc
var translationRender string

//go:embed translate/transduce.alc
var translationTransduce string

var translationParts = TranslationParts{
	Manifest:  translationManifest,
	Render:    &TranslationPart{Entry: "zon-render", Source: translationRender},
	Transduce: &TranslationPart{Entry: "zon-transduce", Source: translationTransduce},
}

// Translate returns ZON's translation parts.
// Each call returns a copy of its own, so that what one caller changes
// is not what another reads.
func Translate() *TranslationParts {
	parts := translationParts
	parts.Lift = copyPart(parts.Lift)
	parts.Embed = copyPart(parts.Embed)
	parts.Render = copyPart(parts.Render)
	parts.Transduce = copyPart(parts.Transduce)
	return &parts
}

// copyPart is a part of its own, so that no caller reaches another's.
func copyPart(part *TranslationPart) *TranslationPart {
	if part == nil {
		return nil
	}
	copied := *part
	return &copied
}
