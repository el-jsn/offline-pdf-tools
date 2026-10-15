// Read saved values, rather than widget appearance streams. Names identify a
// logical field across PDFs; object IDs can change when a document is saved.
export function comparisonFields(annotations, page) {
  return annotations.filter((item) => item.subtype === 'Widget' &&
    ['Tx', 'Ch', 'Btn'].includes(item.fieldType) && !item.pushButton).map((item) => {
    const name = item.fieldName || 'Unnamed field';
    const key = item.fieldName || `unnamed:${page}:${JSON.stringify(item.rect)}`;
    const values = Array.isArray(item.fieldValue) ? item.fieldValue : [item.fieldValue ?? ''];
    const value = item.checkBox
      ? (!item.fieldValue || item.fieldValue === 'Off' ? 'Unchecked' : 'Checked')
      : values.map((value) => String(value).normalize('NFC')).join('\n');
    return { key, name, type: item.fieldType, value, page };
  });
}

export function compareFields(original, revised) {
  const group = (fields) => {
    const result = new Map();
    for (const field of fields) {
      const key = JSON.stringify([field.key, field.type]);
      if (!result.has(key)) result.set(key, { ...field, values: new Set() });
      result.get(key).values.add(field.value);
    }
    return result;
  };
  const left = group(original), right = group(revised), changes = [];
  for (const key of new Set([...left.keys(), ...right.keys()])) {
    const oldField = left.get(key), newField = right.get(key);
    const values = (field) => field ? [...field.values].sort() : null;
    if (JSON.stringify(values(oldField)) === JSON.stringify(values(newField))) continue;
    const display = (field) => field ? values(field).map((value) => value || 'Empty') : ['Field not present'];
    changes.push({ kind: 'field', before: `Form field: ${(newField || oldField).name}`,
      oldPage: oldField?.page ?? null, newPage: newField?.page ?? null,
      removed: display(oldField), added: display(newField) });
  }
  return changes;
}
