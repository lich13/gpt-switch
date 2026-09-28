import {useState} from "react";
import type {Dashboard,Tokens} from "./usage-types";
import {number,money,date} from "./usage-types";
const colors=["#5CE1E6","#2B7FFF","#8B5CF6","#F3AE5B"];
export default function UsageChart({data,zoom}:{data:Dashboard;zoom:(a:number,b:number)=>void}) {
 const [mode,setMode]=useState("requests"),[hover,setHover]=useState<number|null>(null),[from,setFrom]=useState<number|null>(null);
 const rows=data.trends;
 const keys=mode==="tokens"?["input","output","cacheRead","cacheWrite"]:[mode];
 const value=(i:number,key:string)=>key==="requests"?rows[i].requests:key==="cost"?Number(rows[i].cost??0):rows[i].tokens[key as keyof Tokens]??0;
 const max=Math.max(1,...rows.flatMap((_,i)=>keys.map(k=>value(i,k))));
 const x=(i:number)=>40+i/Math.max(1,rows.length-1)*690,y=(n:number)=>160-n/max*136;
 const index=(e:React.PointerEvent<SVGSVGElement>)=>Math.min(rows.length-1,Math.max(0,Math.round(((e.clientX-e.currentTarget.getBoundingClientRect().left)/e.currentTarget.getBoundingClientRect().width*760-40)/690*Math.max(1,rows.length-1))));
 return <section className="usage-card trend-card"><div className="usage-section-heading"><h2>用量趋势</h2><div className="segmented">{[["requests","请求"],["tokens","Token"],["cost","估算费用"]].map(([k,n])=><button key={k} aria-pressed={mode===k} onClick={()=>setMode(k)}>{n}</button>)}</div></div>
 {rows.length?<><svg viewBox="0 0 760 194" role="img" aria-label="用量趋势，拖动选择时间范围" onPointerMove={e=>setHover(index(e))} onPointerLeave={()=>{if(from===null)setHover(null);}} onPointerDown={e=>{setFrom(index(e));e.currentTarget.setPointerCapture(e.pointerId);}} onPointerUp={e=>{const to=index(e);if(from!==null&&to!==from)zoom(rows[Math.min(from,to)].at,rows[Math.max(from,to)].at+(data.precision==="hour"?3600:86400));setFrom(null);}}>
 {[0,.5,1].map(f=><g key={f}><line x1="40" x2="730" y1={y(f*max)} y2={y(f*max)} stroke="var(--line)"/><text x="32" y={y(f*max)+4} textAnchor="end">{number(f*max)}</text></g>)}
 {keys.map((k,c)=><polyline key={k} fill="none" stroke={colors[c]} strokeWidth="2.5" strokeLinejoin="round" points={rows.map((_,i)=>`${x(i)},${y(value(i,k))}`).join(" ")}/>)}
 {hover!==null&&rows[hover]&&<line x1={x(hover)} x2={x(hover)} y1="20" y2="162" stroke="var(--muted)" strokeDasharray="3 3"/>}
 {from!==null&&hover!==null&&<rect x={Math.min(x(from),x(hover))} y="20" width={Math.abs(x(from)-x(hover))} height="142" fill="#5ce1e620"/>}
 <text x="40" y="186">{date(rows[0].at)}</text><text x="730" y="186" textAnchor="end">{date(rows.at(-1)!.at)}</text></svg>
 <div className="chart-legend">{keys.map((k,c)=><span key={k}><i style={{background:colors[c]}}/>{({input:"输入",output:"输出",cacheRead:"缓存读取",cacheWrite:"缓存写入",requests:"请求",cost:"估算费用"} as Record<string,string>)[k]} {hover!==null&&rows[hover]?(k==="cost"?money(rows[hover].cost):number(value(hover,k))):""}</span>)}{hover!==null&&rows[hover]&&<time>{date(rows[hover].at)}</time>}</div></>:<div className="usage-empty">当前范围没有请求</div>}
 </section>;
}
