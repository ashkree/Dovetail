import subprocess, json, time, threading
p = subprocess.Popen(["claude","-p","--input-format","stream-json","--output-format","stream-json","--verbose","--model","haiku","--effort","low"],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, cwd="/tmp/dt-model-spike")
def send(o): p.stdin.write(json.dumps(o)+"\n"); p.stdin.flush()
out=[]
def rd():
    for l in p.stdout: out.append(l)
threading.Thread(target=rd,daemon=True).start()
send({"type":"control_request","request_id":"r1","request":{"subtype":"initialize"}})
time.sleep(4)
send({"type":"control_request","request_id":"r2","request":{"subtype":"list_models"}})
send({"type":"control_request","request_id":"r3","request":{"subtype":"set_model","model":"sonnet"}})
send({"type":"control_request","request_id":"r4","request":{"subtype":"apply_flag_settings","settings":{"effortLevel":"high"}}})
send({"type":"control_request","request_id":"r5","request":{"subtype":"set_model","model":"bogus-model-xyz"}})
send({"type":"control_request","request_id":"r6","request":{"subtype":"apply_flag_settings","settings":{"effortLevel":"bogus"}}})
send({"type":"control_request","request_id":"r7","request":{"subtype":"get_settings"}})
time.sleep(4)
send({"type":"user","message":{"role":"user","content":"Reply with just: ok"}})
t=time.time()
while time.time()-t<60 and not any('"type":"result"' in l for l in out): time.sleep(0.5)
p.stdin.close(); p.wait(timeout=20)
for l in out:
    o=json.loads(l)
    if o.get("type") in ("control_response","result") or o.get("subtype")=="init" or o.get("type")=="assistant":
        print(json.dumps(o)[:2500]); print()
