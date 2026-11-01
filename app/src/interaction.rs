//! Small, bounded local interaction feedback. No queued gestures or audio assets.
#[derive(Default)]
pub struct PadFilter {steps:u8,last_code:Option<u64>,last_input:Option<u64>,last_sent:Option<u64>}
impl PadFilter {
    pub fn reset(&mut self){*self=Self::default();}
    pub fn accept(&mut self,code:u64,now:u64,slow:bool)->bool{
        if !slow{self.reset();return true;}
        if self.last_code!=Some(code)||self.last_input.is_none_or(|t|now.saturating_sub(t)>200){self.steps=0;}
        self.last_code=Some(code);self.last_input=Some(now);self.steps+=1;
        if self.steps<2{return false;}self.steps=0;
        if self.last_sent.is_some_and(|t|now.saturating_sub(t)<100){return false;}
        self.last_sent=Some(now);true
    }
}

const TONE_FRAMES:usize=882; // 20 ms, including an envelope at both ends.
#[derive(Default)]
pub struct Tone {position:usize,active:bool,last:Option<u64>,click:bool}
impl Tone {
    pub fn active(&self)->bool{self.active}
    pub fn cancel(&mut self){self.active=false;self.position=0;}
    pub fn trigger(&mut self,now:u64,click:bool)->bool{
        if self.last.is_some_and(|t|now.saturating_sub(t)<120){return false;}
        self.last=Some(now);self.position=0;self.click=click;self.active=true;true
    }
    pub fn mix(&mut self,pcm:&mut [u8]){
        for frame in pcm.chunks_exact_mut(4){
            if !self.active{break;}
            let i=self.position;
            let envelope=(i.min(TONE_FRAMES-1-i).min(132)as f64)/132.0;
            let frequency=if self.click{1100.0}else{800.0};
            let sample=((i as f64*frequency*std::f64::consts::TAU/44100.0).sin()*envelope*700.0)as i32;
            for channel in frame.chunks_exact_mut(2){let input=i16::from_le_bytes([channel[0],channel[1]])as i32;let output=(input+sample).clamp(i16::MIN as i32,i16::MAX as i32)as i16;channel.copy_from_slice(&output.to_le_bytes());}
            self.position+=1;if self.position==TONE_FRAMES{self.active=false;}
        }
    }
}

#[cfg(test)]mod tests {
    use super::*;
    #[test]fn slow_pad_halves_steps_and_discards_bursts_without_replay(){
        let mut p=PadFilter::default();assert!(!p.accept(4,0,true));assert!(p.accept(4,10,true));
        for t in 11..20{assert!(!p.accept(4,t,true));}
        p.reset();assert!(!p.accept(3,400,true));assert!(p.accept(3,420,true));
        assert!(p.accept(3,421,false));assert!(p.accept(4,422,false));
    }
    #[test]fn feedback_is_short_stereo_ramped_and_has_no_tail_or_queue(){
        let mut tone=Tone::default();assert!(tone.trigger(0,false));assert!(!tone.trigger(1,true));
        let mut pcm=vec![0u8;4*(TONE_FRAMES+10)];tone.mix(&mut pcm);
        assert!(!tone.active());assert!(pcm.iter().any(|b|*b!=0));assert_eq!(&pcm[..4],&[0;4]);assert_eq!(&pcm[4*(TONE_FRAMES-1)..],&[0;44]);
        for frame in pcm.chunks_exact(4){assert_eq!(&frame[..2],&frame[2..]);}
        assert!(tone.trigger(200,true));tone.cancel();let mut silence=[0u8;40];tone.mix(&mut silence);assert_eq!(silence,[0;40]);
    }
}
