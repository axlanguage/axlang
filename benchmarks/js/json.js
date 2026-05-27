const payload =
  '{ "task" : "summarize", "ok" : true, "tokens" : 2048, "agent" : { "name" : "codex", "limits" : { "tokens" : 2048 } }, "steps" : [{ "name" : "read" }, { "name" : "verify" }], "tools" : ["read", "write", "verify"] }';

function kind(value) {
  if (value === null) return 'null';
  if (Array.isArray(value)) return 'array';
  if (typeof value === 'boolean') return 'bool';
  if (typeof value === 'number') return 'number';
  if (typeof value === 'string') return 'string';
  if (typeof value === 'object') return 'object';
  return 'missing';
}

function jsonScore(n) {
  let acc = 0;
  for (let i = 0; i < n; i += 1) {
    const data = JSON.parse(payload);
    if (Object.prototype.hasOwnProperty.call(data, 'task') && data.ok === true) {
      const task = data.task;
      const agent = data.agent.name;
      const firstStep = data.steps[0].name;
      const taskKind = kind(task);
      const nestedKind = kind(data.agent.limits.tokens);
      const toolCount = data.tools.length;
      const firstTool = data.tools[0];
      const tokens = data.agent.limits.tokens;
      if (tokens === 2048 && data.tools.includes('verify') && Object.prototype.hasOwnProperty.call(data, 'steps')) {
        acc += task.length;
        acc += agent.length;
        acc += firstStep.length;
        acc += taskKind.length;
        acc += nestedKind.length;
        acc += firstTool.length;
        acc += toolCount;
        acc += tokens % 97;
      }
    }
  }
  return acc % 251;
}

process.exit(jsonScore(10000));
