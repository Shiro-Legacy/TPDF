# SPDX-License-Identifier: AGPL-3.0-or-later
"""R24 independent source-affine image composition with frozen R22 certificates.

Native callbacks supply paint order, unscaled decoded pixels, and unhinted glyph/
path geometry. Original PDF dictionaries and native-decoded content spans supply
binary64 source image transforms; native f32 placement is checked, never used as
an exact source-matrix substitute. Strict direct-dictionary fixture grammar only.
"""
from fractions import Fraction as F
from math import ceil, floor, isfinite, lcm
import re
from page_raster_review_model import segments, sample_mask
from stroke_review_model import StrokeModel, Budget

NUM=rb'[+-]?(?:\d+(?:\.\d*)?|\.\d+)'
IDENTITY=(1.,0.,0.,1.,0.,0.)
def concat(left,right):
    # Generic homogeneous multiplication, independent of production typed Affine.
    def rows(m):
        a,b,c,d,e,f=m
        return ((a,c,e),(b,d,f),(0.,0.,1.))
    a,b=rows(left),rows(right)
    out=[[sum(b[i][k]*a[k][j] for k in range(3)) for j in range(3)] for i in range(3)]
    return (out[0][0],out[1][0],out[0][1],out[1][1],out[0][2],out[1][2])
def dictionaries(data):
    found={}
    for match in re.finditer(rb'(?m)^(\d+)\s+(\d+)\s+obj\b',data):
        key=(int(match[1]),int(match[2]))
        if key in found:raise ValueError('source dictionary duplicate object')
        tail=data[match.end():]
        end=re.search(rb'\b(?:stream|endobj)\b',tail)
        if not end:raise ValueError('source object terminator')
        found[key]=tail[:end.start()]
    return found

def array(dictionary,key,count,default=None):
    token=re.search(rb'/'+key.encode()+rb'\b',dictionary)
    if token is None:return default
    match=re.match(rb'\s*\[([^]]*)\]',dictionary[token.end():])
    if not match:raise ValueError('source array outside direct fixture profile: '+key)
    values=match[1].split()
    if len(values)!=count or not all(re.fullmatch(NUM,v) for v in values):raise ValueError('source numeric array: '+key)
    result=tuple(float(v) for v in values)
    if not all(map(isfinite,result)):raise ValueError('source nonfinite numeric array')
    return result

def scalar(dictionary,key,default=None):
    token=re.search(rb'/'+key.encode()+rb'\b',dictionary)
    if token is None:return default
    match=re.match(rb'\s*('+NUM+rb')(?=\s|/|>)',dictionary[token.end():])
    if not match:raise ValueError('source scalar outside direct fixture profile: '+key)
    rest=dictionary[token.end()+match.end():]
    if re.match(rb'\s+\d+\s+R\b',rest):raise ValueError('indirect source scalar unsupported')
    value=float(match[1])
    if not isfinite(value):raise ValueError('nonfinite source scalar')
    return value

def source_geometry(page,objects):
    origins=page['operation_origins']
    roots=[o['object'] for o in origins if o['invocation'] is None]
    if not roots:raise ValueError('source geometry lacks root page operations')
    node=(roots[0],0);seen=set();media=crop=rotation=None;unit=None
    while node is not None:
        if node in seen or len(seen)>=64:raise ValueError('source page ancestry')
        seen.add(node);d=objects[node]
        if unit is None:unit=scalar(d,'UserUnit',1.)
        if media is None:media=array(d,'MediaBox',4)
        if crop is None:crop=array(d,'CropBox',4)
        if rotation is None:rotation=scalar(d,'Rotate')
        parent=re.search(rb'/Parent\s+(\d+)\s+(\d+)\s+R\b',d)
        node=(int(parent[1]),int(parent[2])) if parent else None
    if media is None:raise ValueError('source MediaBox missing')
    crop=crop or media
    x0,y0,x1,y1=max(media[0],crop[0]),max(media[1],crop[1]),min(media[2],crop[2]),min(media[3],crop[3])
    angle=int(rotation or 0)%360
    if angle not in (0,90,180,270) or x0>=x1 or y0>=y1:raise ValueError('source page geometry')
    a,b,c,d={0:(1.,0.,0.,-1.),90:(0.,1.,1.,0.),180:(-1.,0.,0.,1.),270:(0.,-1.,-1.,0.)}[angle]
    corners=[(a*x+c*y,b*x+d*y) for x,y in [(x0,y0),(x1,y0),(x1,y1),(x0,y1)]]
    lowx,lowy=min(p[0] for p in corners),min(p[1] for p in corners)
    matrix=(a*unit,b*unit,c*unit,d*unit,-lowx*unit,-lowy*unit)
    bounds=(0.,0.,(max(p[0] for p in corners)-lowx)*unit,(max(p[1] for p in corners)-lowy)*unit)
    for actual,want in zip(page['initial_ctm'],matrix):
        if abs(actual-want)>1e-4*max(1,abs(want)):raise ValueError('native/source initial matrix disagreement')
    return matrix,bounds

def source_images(page,data,dpi):
    objects=dictionaries(data);current,bounds=source_geometry(page,objects);stack=[];frames=[];result={};fill=(0.,0.,0.);stroke=fill;paints=[];text_colors=set();invocations=page['form_invocations'];root=bytes.fromhex(page['content_hex']);scale=float(dpi)/72
    for index,origin in enumerate(page['operation_origins']):
        phase=origin['phase'];invocation=origin['invocation']
        if phase==1:
            form=invocations[invocation];matrix=array(objects[tuple(form['object'])],'Matrix',6,IDENTITY)
            frames.append((current,stack,fill,stroke));current=concat(matrix,current);stack=[];continue
        if phase==2:
            if stack or not frames:raise ValueError('source Form stack')
            current,stack,fill,stroke=frames.pop();continue
        source=root if invocation is None else bytes.fromhex(invocations[invocation]['source_hex'])
        lo,hi=origin['span'];part=source[lo:hi];op=origin['op']
        if not (0<=lo<hi<=len(source)) or not part.endswith(op.encode()):raise ValueError('native source operation span')
        if op=='q':stack.append((current,fill,stroke))
        elif op=='Q':
            if not stack:raise ValueError('source graphics stack underflow')
            current,fill,stroke=stack.pop()
        elif op=='cm':
            body=re.sub(rb'%[^\r\n]*',b'',part[:-2]);tokens=body.split()
            if len(tokens)!=6 or not all(re.fullmatch(NUM,v) for v in tokens):raise ValueError('source cm shape')
            current=concat(tuple(float(v) for v in tokens),current)
        elif op in ('g','G','rg','RG'):
            body=re.sub(rb'%[^\r\n]*',b'',part[:-len(op)]);tokens=body.split()
            if len(tokens)!=(1 if len(op)==1 else 3) or not all(re.fullmatch(NUM,v) for v in tokens):raise ValueError('source color operands')
            values=tuple(max(0.,min(1.,float(v))) for v in tokens)
            if len(values)==1:values*=3
            if op in ('g','rg'):fill=values
            else:stroke=values
        elif op in ('k','K'):raise ValueError('source CMYK outside certificate')
        elif op in ('f','F','f*'):paints.append(('fill',fill))
        elif op in ('S','s'):paints.append(('stroke',stroke))
        elif op in ('B','B*','b','b*'):paints.extend([('fill',fill),('stroke',stroke)])
        elif op in ('Tj','TJ',"'",'"'):text_colors.add(fill)
        elif op=='Do':result[index]=tuple(F(v*scale) for v in current)
    if frames or stack:raise ValueError('source unbalanced graphics state')
    events=[e for e in page['events'] if e['kind']=='image']
    if len(events)!=len(result):raise ValueError('source image callback count')
    for event in events:
        matrix=result[event['operation_index']];a,b,c,d,e,f=map(float,matrix)
        # Native callbacks map top-down image coordinates. Source Do uses PDF v-up.
        callback=[v*scale for v in event['matrix']];expected=[a,b,-c,-d,c+e,d+f]
        if any(abs(x-y)>1e-4*max(1,abs(y)) for x,y in zip(callback,expected)):raise ValueError('native/source image matrix disagreement')
    colors={};paint_index=0
    if len(text_colors)>1:raise ValueError('multiple source text colors require per-glyph source association')
    text_color=next(iter(text_colors),None)
    for index,event in enumerate(page['events']):
        kind=event['kind']
        if kind in ('fill','stroke'):
            if paint_index>=len(paints) or paints[paint_index][0]!=kind:raise ValueError('native/source path paint order')
            values=paints[paint_index][1];paint_index+=1;native=event['color']
        elif kind=='glyph':
            if event['mode']==3:continue
            if text_color is None:raise ValueError('source text color missing')
            values=text_color;native=page['contexts'][event['context']]['color']
        else:continue
        actual=tuple(native['values'])*3 if native['space']=='gray' else tuple(native['values'])
        if len(actual)!=3 or any(abs(a-b)>1e-6 for a,b in zip(actual,values)):raise ValueError('native/source paint color disagreement')
        colors[index]=bytes(floor(v*255+.5) for v in values)
    if paint_index!=len(paints):raise ValueError('missing native source paints')
    return result,tuple(F(v*scale) for v in bounds),colors

def window(points,w,h,margin=F(0)):
    if not points:return (0,0,0,0)
    xs=[p[0] for p in points];ys=[p[1] for p in points]
    return max(0,floor(min(xs)-margin)-1),max(0,floor(min(ys)-margin)-1),min(w,ceil(max(xs)+margin)+1),min(h,ceil(max(ys)+margin)+1)

def image_mapping(matrix,width,height):
    # Solve the two linear equations by Gaussian elimination, then scale each
    # inverse row to integers. No determinant numerator or production limbs.
    a,b,c,d,e,f=matrix
    if a:
        pivot=d-b*c/a
        if not pivot:raise ValueError('singular image matrix')
        v=(-b/a/pivot,1/pivot,(b*e/a-f)/pivot)
        u=(1/a-c*v[0]/a,-c*v[1]/a,-e/a-c*v[2]/a)
    else:
        if not b or not c:raise ValueError('singular image matrix')
        v=(1/c,F(0),-e/c);u=(-d*v[0]/b,1/b,(-f-d*v[2])/b)
    def integer_row(row):
        denominator=lcm(*(v.denominator for v in row),8)
        return tuple(int(v*denominator) for v in row)+(denominator,)
    # row source coordinate is height*(1-v), before floor.
    return integer_row(tuple(width*x for x in u)),integer_row((-height*v[0],-height*v[1],height*(1-v[2])))

def image_samples(event,matrix,w,h):
    a,b,c,d,e,f=matrix;polygon=[(e,f),(a+e,b+f),(a+c+e,b+d+f),(c+e,d+f)]
    box=window(polygon,w,h);x0,y0,x1,y1=box
    raw=bytes.fromhex(event['samples_hex']);iw,ih,components=event['width'],event['height'],event['components']
    if iw<=0 or ih<=0 or components not in (1,3) or len(raw)!=iw*ih*components:raise ValueError('native image sample length')
    denominator=lcm(8,*(v.denominator for p in polygon for v in p))
    points=[tuple(int(v*denominator) for v in p) for p in polygon]
    edges=list(zip(points,points[1:]+points[:1]));u,v=image_mapping(matrix,iw,ih)
    for sy in range(y0*4,y1*4):
        py=(2*sy+1)*(denominator//8)
        for sx in range(x0*4,x1*4):
            px=(2*sx+1)*(denominator//8);inside=False
            # Half-open horizontal ray crossings on exact common-scale integers.
            for (ax,ay),(bx,by) in edges:
                if (ay>py)!=(by>py):
                    cross=(bx-ax)*(py-ay)-(px-ax)*(by-ay)
                    if (cross>0 if by>ay else cross<0):inside=not inside
            if not inside:continue
            def lookup(row,dimension):
                A,B,C,D=row;num=A*(2*sx+1)+B*(2*sy+1)+8*C;den=8*D
                result=num//den
                if result==dimension and num==dimension*den:return dimension-1
                if not 0<=result<dimension:raise ValueError('source pixel outside admitted footprint')
                return result
            col,row=lookup(u,iw),lookup(v,ih);offset=(row*iw+col)*components;color=raw[offset:offset+components]
            yield sy*(w*4)+sx,color*3 if components==1 else color

def rgb(color):
    if color['space'] not in ('gray','rgb'):raise ValueError('model color profile')
    values=[int(F(str(v))*255+F(1,2)) for v in color['values']]
    if color['space']=='gray':values*=3
    if len(values)!=3 or any(not 0<=v<=255 for v in values):raise ValueError('model color range')
    return bytes(values)

def geometry_mask(event,w,h,scale,stats):
    matrix=tuple(F(str(v))*scale for v in event.get('matrix',IDENTITY));path=event['path'];stroke=event['kind']=='stroke'
    transformed=segments(path,matrix,snap_lines=not stroke and all(c[0] in ('m','l','h') for c in path))
    points=[p for edge in transformed for p in edge];margin=F(1,32)
    if stroke:
        style=event['style']
        if style['dash'] or style['width']<=0 or style['start_cap']!=style['end_cap'] or style['start_cap']!=style['dash_cap']:raise ValueError('stroke profile')
        norm=max(abs(matrix[0])+abs(matrix[2]),abs(matrix[1])+abs(matrix[3]))
        margin+=F(str(style['width']))/2*max(2,F(str(style['miter'])) if style['join']==0 else 2)*norm
    x0,y0,x1,y1=window(points,w,h,margin)
    if x0>=x1 or y0>=y1:return
    cw,ch=x1-x0,y1-y0
    if stroke:
        shifted=list(matrix);shifted[4]-=x0;shifted[5]-=y0
        model=StrokeModel(path,style['width'],cap=['butt','round','square'][style['start_cap']],join=['miter','round','bevel'][style['join']],miter_limit=style['miter'],matrix=shifted,budget=Budget(max_work=128_000_000,max_nodes=500_000,max_queries=4_000_000))
        for y in range(ch*4):
            for x in range(cw*4):
                value=model.contains((F(2*x+1,8),F(2*y+1,8)),F(1,32))
                if value is not False:yield (y+4*y0)*(4*w)+x+4*x0,value is True,value is None
        stats['stroke_queries']+=model.b.queries;stats['stroke_work']+=model.b.work
    else:
        shifted=[tuple((x-x0,y-y0) for x,y in edge) for edge in transformed]
        mask,ambiguous=sample_mask(shifted,event.get('rule','nonzero'),cw,ch)
        for y in range(ch*4):
            for x in range(cw*4):
                local=y*cw*4+x
                if mask[local] or ambiguous[local]:yield (y+4*y0)*(4*w)+x+4*x0,mask[local],ambiguous[local]
        stats['geometry_sample_queries']+=cw*ch*16

def certify(page,w,h,dpi=72,source_pdf=None):
    if source_pdf is None:raise ValueError('original source PDF required for image matrix certificate')
    if page['status']!='accepted' or not 0<w*h<=2_000_000:raise ValueError('model page bound/status')
    mask_count = validate_mask_callbacks(page, source_pdf)
    glyph_paths = source_glyph_paths(page, source_pdf) if any(e["kind"] == "glyph" for e in page["events"]) else {}
    matrices,bounds,colors=source_images(page,source_pdf,dpi);n=w*h*16;stride=w*4;scale=F(dpi,72)
    if (w,h)!=(ceil(bounds[2]),ceil(bounds[3])):raise ValueError('source model canvas extent')
    stats=dict(opaque_masks=mask_count,images=0,source_ctm_checks=len(matrices),source_color_checks=len(colors),source_glyph_checks=len(glyph_paths),stroke_queries=0,stroke_work=0,geometry_sample_queries=0,image_covered_samples=0,pixels=w*h,samples=n)
    pixels=bytearray([255])*(3*n);unknown=bytearray(n);base=bytearray(n)
    # Exact integer sample ranges: low <= (2*s+1)/8 < high.
    sx0=max(0,ceil((8*bounds[0]-1)/2));sx1=min(4*w,ceil((8*bounds[2]-1)/2))
    sy0=max(0,ceil((8*bounds[1]-1)/2));sy1=min(4*h,ceil((8*bounds[3]-1)/2))
    for sy in range(sy0,sy1):base[sy*stride+sx0:sy*stride+sx1]=bytes([1])*max(0,sx1-sx0)
    stack=[(base,bytearray(n))];clip_ids=[]
    for event_index,original in enumerate(page['events']):
        event=original;kind=event['kind']
        if kind=='pop':
            if len(stack)==1:raise ValueError('model clip underflow')
            stack.pop();clip_ids.pop();continue
        if event.get('clip_ids',clip_ids)!=clip_ids:raise ValueError('native callback clip ancestry')
        if kind=='glyph':
            if event['mode']==3:continue
            if event['mode']!=0:raise ValueError('native glyph paint mode')
            context=page['contexts'][event['context']]
            if context['clip_ids']!=clip_ids:raise ValueError('native glyph clip ancestry')
            event=dict(kind='fill',path=glyph_paths[event_index],matrix=IDENTITY,color=context['color'],rule='nonzero');kind='fill'
        parent,parent_unknown=stack[-1]
        if kind=='image':
            stats['images']+=1
            for index,color in image_samples(event,matrices[event['operation_index']],w,h):
                stats['image_covered_samples']+=1
                if parent[index]:pixels[3*index:3*index+3]=color
                if parent[index] and not parent_unknown[index]:unknown[index]=0
                elif parent_unknown[index]:unknown[index]=1
            continue
        if kind=='opaque_mask_clip':
            stack.append((parent,parent_unknown));clip_ids.append(event_index);continue
        if kind=='clip':
            mask,uncertain=bytearray(n),bytearray(n)
            for index,inside,ambiguous in geometry_mask(event,w,h,scale,stats):
                mask[index]=bool(parent[index] and inside)
                uncertain[index]=bool((parent_unknown[index] and (inside or ambiguous)) or (parent[index] and ambiguous))
            stack.append((mask,uncertain));clip_ids.append(event_index);continue
        if kind not in ('fill','stroke'):raise ValueError('unknown model paint kind')
        color=colors[event_index]
        for index,inside,ambiguous in geometry_mask(event,w,h,scale,stats):
            if parent[index] and inside:pixels[3*index:3*index+3]=color
            if parent[index] and inside and not (parent_unknown[index] or ambiguous):unknown[index]=0
            elif (parent_unknown[index] and (inside or ambiguous)) or (parent[index] and ambiguous):unknown[index]=1
    if len(stack)-1!=page['cleanup_pops']:raise ValueError('native cleanup clip depth')
    out=bytearray();band=[];channel_bounds=[]
    for y in range(h):
        for x in range(w):
            ids=[(4*y+j)*stride+4*x+i for j in range(4) for i in range(4)];uncertain=sum(unknown[k] for k in ids);band.append(bool(uncertain))
            for c in range(3):
                out.append((sum(pixels[3*k+c] for k in ids)+8)//16)
                known=sum(pixels[3*k+c] for k in ids if not unknown[k]);channel_bounds.append(((known+8)//16,(known+255*uncertain+8)//16))
    stats.update(uncertain_samples=sum(unknown),uncertain_pixels=sum(band));stats['_channel_bounds']=channel_bounds
    return bytes(out),band,stats

def mixed_cases():
    from image_decode_cases import pdf,stream
    raw=bytes([255,0,0,0,255,0,0,0,255,255,255,0])
    for label,content in [
        ('text-image-stroke',b'BT /F 10 Tf 1 3 Td (A) Tj ET q 8 0 0 8 4 4 cm /Im Do Q 0 0 0 RG 1 w 2 2 m 14 14 l S'),
        ('image-text-stroke-image',b'q 10 0 0 10 2 2 cm /Im Do Q BT /F 10 Tf 1 3 Td (A) Tj ET 0 0 0 RG 1 w 2 12 m 14 2 l S q 3 0 0 3 6 5 cm /Im Do Q')]:
        data=pdf([b'<< /Type /Catalog /Pages 2 0 R >>',b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 16 16] /Resources << /XObject << /Im 5 0 R >> /Font << /F 6 0 R >> >> /Contents 4 0 R >>',stream(content),stream(raw,'/Type /XObject /Subtype /Image /Width 2 /Height 2 /BitsPerComponent 8 /ColorSpace /DeviceRGB'),b'<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>'])
        yield dict(label=label,pdf=data,width=16,height=16)


def source_alpha(data, object_id):
    """Independent direct-dictionary identity/Flate decoder for original corpus masks."""
    import zlib
    match=re.search(rb'(?m)^'+str(object_id[0]).encode()+rb' '+str(object_id[1]).encode()+rb' obj\s*(<<.*?)stream\r?\n',data,re.S)
    if not match:raise ValueError('source mask object missing')
    d=match[1]
    if not re.search(rb'/ColorSpace\s*/DeviceGray\b',d) or scalar(d,'BitsPerComponent')!=8:raise ValueError('source Gray8 mask')
    if any(re.search(rb'/'+key+rb'\b',d) for key in [b'SMask',b'Mask',b'Matte',b'TR',b'Interpolate',b'DecodeParms']):raise ValueError('source mask outside independent decoder profile')
    n=scalar(d,'Length');w=scalar(d,'Width');h=scalar(d,'Height')
    if any(x is None or int(x)!=x or x<=0 for x in [n,w,h]):raise ValueError('source mask dimensions/length')
    raw=data[match.end():match.end()+int(n)]
    if len(raw)!=n:raise ValueError('source mask truncation')
    f=re.search(rb'/Filter\s*/(\w+)',d)
    if f:
        if f[1]!=b'FlateDecode':raise ValueError('source mask filter')
        dec=zlib.decompressobj();raw=dec.decompress(raw,4_194_305)
        if not dec.eof or dec.unused_data or dec.unconsumed_tail:raise ValueError('source mask compressed envelope')
    if len(raw)!=w*h or len(raw)>4_194_304:raise ValueError('source mask sample length')
    decode=array(d,'Decode',2,(0.,1.))
    if decode==(1.,0.):raw=bytes(255-x for x in raw)
    elif decode!=(0.,1.):raise ValueError('source mask Decode')
    return int(w),int(h),raw

def validate_mask_callbacks(page,data):
    objects=dictionaries(data);events=page['events'];count=0;seen=set()
    for i,e in enumerate(events):
        if e['kind']!='image':continue
        d=objects[tuple(e['object'])];edge=re.search(rb'/SMask\s+(\d+)\s+(\d+)\s+R\b',d)
        if not edge:
            if i and events[i-1]['kind']=='opaque_mask_clip':raise ValueError('mask callback without source edge')
            continue
        mask_id=[int(edge[1]),int(edge[2])]
        if i==0 or i+1>=len(events):raise ValueError('missing mask callbacks')
        clip=events[i-1];pop=events[i+1]
        if clip['kind']!='opaque_mask_clip' or pop['kind']!='pop' or clip['base_object']!=e['object'] or clip['mask_object']!=mask_id:raise ValueError('mask clip/fill/pop identity')
        w,h,alpha=source_alpha(data,mask_id)
        if (w,h)!=(e['width'],e['height']) or (w,h)!=(clip['width'],clip['height']):raise ValueError('mask source/callback shape')
        if bytes.fromhex(clip['alpha_hex'])!=alpha or any(v!=255 for v in alpha):raise ValueError('source/attached opacity proof')
        if clip['matrix']!=e['matrix'] or e['clip_ids']!=clip['clip_ids']+[i-1] or pop['clip_ids']!=clip['clip_ids']:raise ValueError('mask clip placement/ancestry')
        count+=1;seen.add(i-1)
    if {i for i,e in enumerate(events) if e['kind']=='opaque_mask_clip'}!=seen:raise ValueError('orphan mask callbacks')
    return count

def source_glyph_paths(page,data):
    """Strict ASCII/Helvetica Tj source witness; refuse unsupported text syntax.

    Original text operands and pinned AFM integer widths determine positions.
    Native doubled unhinted CFF coordinates determine outlines, and their native
    placement is checked. No Rust glyph coordinates enter this certificate.
    """
    from pathlib import Path
    import hashlib
    afm=(Path(__file__).resolve().parents[1]/'reference/soft-mask-Helvetica-r24.afm').read_bytes()
    if hashlib.sha256(afm).hexdigest()!='ed4ead49b4d090c80c1d4a8d771879153a41af262d331168dd2635508634cfa1':raise ValueError('AFM identity')
    widths={m[2].decode():int(m[1]) for m in re.finditer(rb'WX (\d+) ; N (\w+) ;',afm)}
    objects=dictionaries(data);initial,_=source_geometry(page,objects);current=tuple(map(F,initial));stack=[];frames=[];size=None;tx=ty=F(0);in_text=False
    def multiply(l,r):
        a,b,c,d,e,f=l;A,B,C,D,E,G=r
        return (A*a+C*b,B*a+D*b,A*c+C*d,B*c+D*d,A*e+C*f+E,B*e+D*f+G)
    result={};glyphs=[(i,e) for i,e in enumerate(page['events']) if e['kind']=='glyph'];cursor=0
    root=bytes.fromhex(page['content_hex'])
    for origin in page['operation_origins']:
        phase=origin['phase'];inv=origin['invocation'];op=origin['op']
        if phase==1:
            form=page['form_invocations'][inv];frames.append((current,stack,size,tx,ty,in_text));current=multiply(tuple(map(F,array(objects[tuple(form['object'])],'Matrix',6,IDENTITY))),current);stack=[];continue
        if phase==2:
            if in_text or stack:raise ValueError('source glyph Form state')
            current,stack,size,tx,ty,in_text=frames.pop();continue
        src=root if inv is None else bytes.fromhex(page['form_invocations'][inv]['source_hex']);lo,hi=origin['span'];part=src[lo:hi];body=part[:-len(op)].strip()
        if op=='q':stack.append((current,size))
        elif op=='Q':current,size=stack.pop()
        elif op=='cm':current=multiply(tuple(F(float(v)) for v in body.split()),current)
        elif op=='BT':
            if in_text:raise ValueError('nested source text')
            in_text=True;tx=ty=F(0)
        elif op=='ET':in_text=False
        elif op=='Tf':
            fields=body.split()
            if len(fields)!=2 or not fields[0].startswith(b'/'):raise ValueError('source Tf shape')
            size=F(float(fields[1]))
        elif op=='Td':
            values=body.split()
            if len(values)!=2:raise ValueError('source Td shape')
            tx+=F(float(values[0]));ty+=F(float(values[1]))
        elif op=='Tj':
            if not in_text or size is None or not re.fullmatch(rb'\([A-Za-z0-9 ]*\)',body):raise ValueError('source Tj witness profile')
            for code in body[1:-1]:
                if cursor>=len(glyphs):raise ValueError('missing source glyph callback')
                index,event=glyphs[cursor];cursor+=1;name=('zero','one','two','three','four','five','six','seven','eight','nine')[code-48] if 48<=code<=57 else (chr(code) if code!=32 else 'space')
                font=objects[tuple(event['font'])]
                if not re.search(rb'/BaseFont\s*/Helvetica\b',font) or not re.search(rb'/Encoding\s*/WinAnsiEncoding\b',font) or re.search(rb'/(?:Widths|Differences)\b',font):raise ValueError('source font width witness profile')
                if event['source_kind']!='pinned_base14' or event['face']!='Helvetica' or event['glyph_name']!=name or event['code']!=code or event['units_per_em']!=1000 or event['asset_sha256']!='366720e054067fa49d030f58005c739fc787dcc9727a9327a28767777791d706':raise ValueError('source glyph identity')
                width=widths[name]
                if abs(event['advance']-width/1000)>1e-6:raise ValueError('native/source width')
                matrix=multiply((size,F(0),F(0),size,tx,ty),current)
                if any(abs(float(x)-y)>1e-4 for x,y in zip(matrix,event['matrix'])):raise ValueError('native/source glyph placement')
                a,b,c,d,e,f=matrix;commands=[]
                for raw,native in zip(event['raw_commands'],event['unhinted_commands']):
                    if raw[0]!=native[0] or len(raw)!=len(native):raise ValueError('native raw glyph command correspondence')
                    out=[raw[0]]
                    for j in range(1,len(raw),2):
                        x,y=F(raw[j],2000),F(raw[j+1],2000);X=a*x+c*y+e;Y=b*x+d*y+f
                        if abs(float(X)-native[j])>1e-4 or abs(float(Y)-native[j+1])>1e-4:raise ValueError('source/raw glyph coordinate')
                        out.extend([X,Y])
                    commands.append(out)
                if len(commands)!=len(event['raw_commands']) or len(commands)!=len(event['unhinted_commands']):raise ValueError('glyph command count')
                result[index]=commands;tx+=F(width,1000)*size
        elif op in ('TJ','Tm','TD','T*','Tc','Tw','Tz','Ts','TL',"'",'"'):raise ValueError('source text operation outside exact witness')
    if cursor!=len(glyphs) or frames or stack or in_text:raise ValueError('source glyph coverage/state')
    return result
