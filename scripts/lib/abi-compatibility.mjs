import { createHash } from "node:crypto";

export function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

export function normalizeSpecText(value) {
  return `${JSON.stringify(JSON.parse(value), null, 2)}\n`;
}

export function extractAbi(spec, interfaceVersion, storageLayoutVersion) {
  const methods = spec
    .filter((entry) => entry.function_v0)
    .map(({ function_v0: method }) => ({
      name: method.name,
      inputs: method.inputs.map(({ name, type }) => ({ name, type })),
      outputs: method.outputs,
    }))
    .sort(byName);
  const events = spec
    .filter((entry) => entry.event_v0)
    .map(({ event_v0: event }) => ({
      name: event.name,
      prefixTopics: event.prefix_topics,
      dataFormat: event.data_format,
      fields: event.params.map(({ name, location, type }) => ({
        name,
        location,
        type,
      })),
    }))
    .sort(byName);
  const error = spec.find((entry) => entry.udt_error_enum_v0)?.udt_error_enum_v0;
  const types = spec
    .filter((entry) => Object.keys(entry)[0]?.startsWith("udt_") && !entry.udt_error_enum_v0)
    .map((entry) => stripDocumentation(entry))
    .sort((left, right) => typeName(left).localeCompare(typeName(right)));

  if (!error) {
    throw new Error("ContractError is missing from the current specification");
  }

  return {
    interfaceVersion,
    storageLayoutVersion,
    methods,
    events,
    errors: error.cases.map(({ name, value }) => ({ name, value })),
    types,
  };
}

export function classifyAbi(baseline, current) {
  const result = { additive: [], compatible: [], breaking: [] };

  compareNamedCollection(
    "method",
    baseline.methods,
    current.methods,
    result,
    (value) => ({ inputs: value.inputs, outputs: value.outputs }),
  );
  compareNamedCollection("event", baseline.events, current.events, result);
  compareNamedCollection(
    "type",
    baseline.types,
    current.types,
    result,
    (value) => value,
    typeName,
  );
  compareErrors(baseline.errors, current.errors, result);

  if (current.interfaceVersion < baseline.interfaceVersion) {
    result.breaking.push(
      `interface version regressed from ${baseline.interfaceVersion} to ${current.interfaceVersion}`,
    );
  } else if (current.interfaceVersion > baseline.interfaceVersion) {
    result.compatible.push(
      `interface version advanced from ${baseline.interfaceVersion} to ${current.interfaceVersion}`,
    );
  }
  if (current.storageLayoutVersion < baseline.storageLayoutVersion) {
    result.breaking.push(
      `storage layout version regressed from ${baseline.storageLayoutVersion} to ${current.storageLayoutVersion}`,
    );
  } else if (current.storageLayoutVersion > baseline.storageLayoutVersion) {
    result.compatible.push(
      `storage layout version advanced from ${baseline.storageLayoutVersion} to ${current.storageLayoutVersion}`,
    );
  }
  if (result.additive.length === 0 && result.breaking.length === 0) {
    result.compatible.push("released ABI remains compatible");
  }

  return result;
}

export function validateBreakingRelease(baseline, current, migration) {
  if (current.interfaceVersion <= baseline.interfaceVersion) {
    return "breaking ABI changes require an increased interface version";
  }
  if (!migration) {
    return "breaking ABI changes require migration evidence";
  }
  if (
    migration.fromInterfaceVersion !== baseline.interfaceVersion ||
    migration.toInterfaceVersion !== current.interfaceVersion ||
    migration.baselineSpecSha256 !== baseline.sourceSpecSha256 ||
    !Array.isArray(migration.compatibilityNotes) ||
    migration.compatibilityNotes.length === 0
  ) {
    return "migration evidence does not match the baseline and current interface versions";
  }
  return null;
}

function compareNamedCollection(
  label,
  baselineValues,
  currentValues,
  result,
  comparable = (value) => value,
  nameOf = (value) => value.name,
) {
  const baseline = new Map(baselineValues.map((value) => [nameOf(value), value]));
  const current = new Map(currentValues.map((value) => [nameOf(value), value]));
  for (const [name, oldValue] of baseline) {
    const newValue = current.get(name);
    if (!newValue) {
      result.breaking.push(`${label} removed: ${name}`);
    } else if (stable(comparable(oldValue)) !== stable(comparable(newValue))) {
      result.breaking.push(`${label} changed: ${name}`);
    }
  }
  for (const name of current.keys()) {
    if (!baseline.has(name)) {
      result.additive.push(`${label} added: ${name}`);
    }
  }
}

function compareErrors(baselineErrors, currentErrors, result) {
  const baselineByName = new Map(baselineErrors.map((value) => [value.name, value]));
  const baselineByValue = new Map(baselineErrors.map((value) => [value.value, value]));
  for (const error of baselineErrors) {
    const current = currentErrors.find((value) => value.name === error.name);
    if (!current) {
      result.breaking.push(`error removed: ${error.name}=${error.value}`);
    } else if (current.value !== error.value) {
      result.breaking.push(
        `error renumbered: ${error.name} ${error.value} -> ${current.value}`,
      );
    }
  }
  for (const error of currentErrors) {
    if (!baselineByName.has(error.name)) {
      if (baselineByValue.has(error.value)) {
        result.breaking.push(
          `error value ${error.value} renamed from ${baselineByValue.get(error.value).name} to ${error.name}`,
        );
      } else {
        result.additive.push(`error added: ${error.name}=${error.value}`);
      }
    }
  }
}

function stripDocumentation(value) {
  if (Array.isArray(value)) {
    return value.map(stripDocumentation);
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .filter(([key]) => key !== "doc" && key !== "lib")
        .map(([key, child]) => [key, stripDocumentation(child)]),
    );
  }
  return value;
}

function typeName(value) {
  const definition = value[Object.keys(value)[0]];
  return definition.name;
}

function stable(value) {
  return JSON.stringify(value);
}

function byName(left, right) {
  return left.name.localeCompare(right.name);
}
