import {useEffect,useRef,type ReactNode} from "react";
import {X} from "lucide-react";
export default function UsageDialog({title,children,close,wide=false}:{title:string;children:ReactNode;close:()=>void;wide?:boolean}) {
 const ref=useRef<HTMLDialogElement>(null);
 useEffect(()=>{const d=ref.current!;d.showModal();return()=>d.close();},[]);
 return <dialog ref={ref} className={`modal ${wide?"usage-wide":""}`} onCancel={e=>{e.preventDefault();close();}}>
  <header className="modal-heading"><h2>{title}</h2><button className="icon-button" aria-label="关闭" onClick={close}><X size={16}/></button></header>
  <div className="modal-content">{children}</div>
 </dialog>;
}
