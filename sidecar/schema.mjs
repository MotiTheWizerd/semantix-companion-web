// JSON Schema → Zod raw shape.
//
// The Rust core declares its tools once, as JSON Schema, for every provider.
// The Agent SDK's `tool()` helper wants a Zod raw shape instead, so this is
// the one place the two vocabularies meet — the declarations themselves stay
// provider-agnostic.
//
// Deliberately narrow: it covers the shapes our tools actually declare
// (string / number / integer / boolean / enum / array / nested object) and
// falls back to a permissive `z.any()` for anything else, so an unmapped
// corner degrades to "unvalidated argument", never to a missing tool.

import { z } from "zod";

function leaf(schema) {
  if (Array.isArray(schema?.enum) && schema.enum.length > 0) {
    // Enum values are strings in every declaration we make; z.enum needs a
    // non-empty tuple, which the length check above guarantees.
    return z.enum(schema.enum.map(String));
  }
  switch (schema?.type) {
    case "string":
      return z.string();
    case "number":
      return z.number();
    case "integer":
      return z.number().int();
    case "boolean":
      return z.boolean();
    case "array":
      return z.array(schema.items ? leaf(schema.items) : z.any());
    case "object":
      return z.object(shapeOf(schema));
    default:
      return z.any();
  }
}

/** The `{ property: ZodType }` shape of one object schema. */
export function shapeOf(schema) {
  const properties = schema?.properties ?? {};
  const required = new Set(schema?.required ?? []);
  const shape = {};
  for (const [name, property] of Object.entries(properties)) {
    let field = leaf(property);
    if (property?.description) field = field.describe(property.description);
    shape[name] = required.has(name) ? field : field.optional();
  }
  return shape;
}
