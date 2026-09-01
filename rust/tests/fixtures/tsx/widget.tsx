// Parity-probe fixture for the Tsx leg (rust/tests/parity_probe.rs). Every rule in
// src/pack/rules/tsx.yml must match something in this file, and the file must be JSX-heavy
// throughout: the grammar is what differs between Ts and Tsx, so a probe file without JSX would
// prove parity for a grammar it never exercised.
import React, { useEffect } from 'react';
import { helper } from './helper';

export function Widget({ label }: { label: string }) {
  const [count, setCount] = React.useState(0);
  useEffect(() => {
    helper(count);
  }, [count]);
  return (
    <button onClick={() => setCount(count + 1)} disabled={count < 0}>
      {label}: {count}
    </button>
  );
}

export class Panel {
  title: string = 'panel';
  render() {
    return (
      <div className="panel">
        <Widget label={this.title} />
        {helper(this.title)}
      </div>
    );
  }
}

export const makePanel = (n: number) => <Panel key={n} title={`panel-${n}`} />;
export const legacy = function (x: string) {
  return helper(x);
};
export const wrapped = create(() => <Widget label="wrapped" />);
