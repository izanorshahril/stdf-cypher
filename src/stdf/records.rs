//! Typed STDF V4 records: decoding from and encoding to raw payloads.

use super::codec::{BitField, Endian, FieldReader, FieldWriter, GenData};
use serde::Serialize;

/// (REC_TYP, REC_SUB) codes of every STDF V4 record.
pub mod code {
    pub const FAR: (u8, u8) = (0, 10);
    pub const ATR: (u8, u8) = (0, 20);
    pub const MIR: (u8, u8) = (1, 10);
    pub const MRR: (u8, u8) = (1, 20);
    pub const PCR: (u8, u8) = (1, 30);
    pub const HBR: (u8, u8) = (1, 40);
    pub const SBR: (u8, u8) = (1, 50);
    pub const PMR: (u8, u8) = (1, 60);
    pub const PGR: (u8, u8) = (1, 62);
    pub const PLR: (u8, u8) = (1, 63);
    pub const RDR: (u8, u8) = (1, 70);
    pub const SDR: (u8, u8) = (1, 80);
    pub const WIR: (u8, u8) = (2, 10);
    pub const WRR: (u8, u8) = (2, 20);
    pub const WCR: (u8, u8) = (2, 30);
    pub const PIR: (u8, u8) = (5, 10);
    pub const PRR: (u8, u8) = (5, 20);
    pub const TSR: (u8, u8) = (10, 30);
    pub const PTR: (u8, u8) = (15, 10);
    pub const MPR: (u8, u8) = (15, 15);
    pub const FTR: (u8, u8) = (15, 20);
    pub const BPS: (u8, u8) = (20, 10);
    pub const EPS: (u8, u8) = (20, 20);
    pub const GDR: (u8, u8) = (50, 10);
    pub const DTR: (u8, u8) = (50, 30);
}

pub fn record_name(typ: u8, sub: u8) -> &'static str {
    match (typ, sub) {
        (0, 10) => "FAR",
        (0, 20) => "ATR",
        (1, 10) => "MIR",
        (1, 20) => "MRR",
        (1, 30) => "PCR",
        (1, 40) => "HBR",
        (1, 50) => "SBR",
        (1, 60) => "PMR",
        (1, 62) => "PGR",
        (1, 63) => "PLR",
        (1, 70) => "RDR",
        (1, 80) => "SDR",
        (2, 10) => "WIR",
        (2, 20) => "WRR",
        (2, 30) => "WCR",
        (5, 10) => "PIR",
        (5, 20) => "PRR",
        (10, 30) => "TSR",
        (15, 10) => "PTR",
        (15, 15) => "MPR",
        (15, 20) => "FTR",
        (20, 10) => "BPS",
        (20, 20) => "EPS",
        (50, 10) => "GDR",
        (50, 30) => "DTR",
        _ => "UNKNOWN",
    }
}

pub fn record_code(name: &str) -> Option<(u8, u8)> {
    use code::*;
    Some(match name.to_ascii_uppercase().as_str() {
        "FAR" => FAR,
        "ATR" => ATR,
        "MIR" => MIR,
        "MRR" => MRR,
        "PCR" => PCR,
        "HBR" => HBR,
        "SBR" => SBR,
        "PMR" => PMR,
        "PGR" => PGR,
        "PLR" => PLR,
        "RDR" => RDR,
        "SDR" => SDR,
        "WIR" => WIR,
        "WRR" => WRR,
        "WCR" => WCR,
        "PIR" => PIR,
        "PRR" => PRR,
        "TSR" => TSR,
        "PTR" => PTR,
        "MPR" => MPR,
        "FTR" => FTR,
        "BPS" => BPS,
        "EPS" => EPS,
        "GDR" => GDR,
        "DTR" => DTR,
        _ => return None,
    })
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Far {
    pub cpu_type: u8,
    pub stdf_ver: u8,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Atr {
    pub mod_tim: u32,
    pub cmd_line: String,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Mir {
    pub setup_t: u32,
    pub start_t: u32,
    pub stat_num: u8,
    pub mode_cod: char,
    pub rtst_cod: char,
    pub prot_cod: char,
    pub burn_tim: u16,
    pub cmod_cod: char,
    pub lot_id: String,
    pub part_typ: String,
    pub node_nam: String,
    pub tstr_typ: String,
    pub job_nam: String,
    pub job_rev: String,
    pub sblot_id: String,
    pub oper_nam: String,
    pub exec_typ: String,
    pub exec_ver: String,
    pub test_cod: String,
    pub tst_temp: String,
    pub user_txt: String,
    pub aux_file: String,
    pub pkg_typ: String,
    pub famly_id: String,
    pub date_cod: String,
    pub facil_id: String,
    pub floor_id: String,
    pub proc_id: String,
    pub oper_frq: String,
    pub spec_nam: String,
    pub spec_ver: String,
    pub flow_id: String,
    pub setup_id: String,
    pub dsgn_rev: String,
    pub eng_id: String,
    pub rom_cod: String,
    pub serl_num: String,
    pub supr_nam: String,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Mrr {
    pub finish_t: u32,
    pub disp_cod: char,
    pub usr_desc: String,
    pub exc_desc: String,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Pcr {
    pub head_num: u8,
    pub site_num: u8,
    pub part_cnt: u32,
    pub rtst_cnt: u32,
    pub abrt_cnt: u32,
    pub good_cnt: u32,
    pub func_cnt: u32,
}

/// HBR and SBR share a layout.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Bin {
    pub head_num: u8,
    pub site_num: u8,
    pub bin_num: u16,
    pub bin_cnt: u32,
    pub bin_pf: char,
    pub bin_nam: String,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Pmr {
    pub pmr_indx: u16,
    pub chan_typ: u16,
    pub chan_nam: String,
    pub phy_nam: String,
    pub log_nam: String,
    pub head_num: u8,
    pub site_num: u8,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Pgr {
    pub grp_indx: u16,
    pub grp_nam: String,
    pub pmr_indx: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Plr {
    pub grp_indx: Vec<u16>,
    pub grp_mode: Vec<u16>,
    pub grp_radx: Vec<u8>,
    pub pgm_char: Vec<String>,
    pub rtn_char: Vec<String>,
    pub pgm_chal: Vec<String>,
    pub rtn_chal: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Rdr {
    pub rtst_bin: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Sdr {
    pub head_num: u8,
    pub site_grp: u8,
    pub site_num: Vec<u8>,
    pub hand_typ: String,
    pub hand_id: String,
    pub card_typ: String,
    pub card_id: String,
    pub load_typ: String,
    pub load_id: String,
    pub dib_typ: String,
    pub dib_id: String,
    pub cabl_typ: String,
    pub cabl_id: String,
    pub cont_typ: String,
    pub cont_id: String,
    pub lasr_typ: String,
    pub lasr_id: String,
    pub extr_typ: String,
    pub extr_id: String,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Wir {
    pub head_num: u8,
    pub site_grp: u8,
    pub start_t: u32,
    pub wafer_id: String,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Wrr {
    pub head_num: u8,
    pub site_grp: u8,
    pub finish_t: u32,
    pub part_cnt: u32,
    pub rtst_cnt: u32,
    pub abrt_cnt: u32,
    pub good_cnt: u32,
    pub func_cnt: u32,
    pub wafer_id: String,
    pub fabwf_id: String,
    pub frame_id: String,
    pub mask_id: String,
    pub usr_desc: String,
    pub exc_desc: String,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Wcr {
    pub wafr_siz: f32,
    pub die_ht: f32,
    pub die_wid: f32,
    pub wf_units: u8,
    pub wf_flat: char,
    pub center_x: i16,
    pub center_y: i16,
    pub pos_x: char,
    pub pos_y: char,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Pir {
    pub head_num: u8,
    pub site_num: u8,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Prr {
    pub head_num: u8,
    pub site_num: u8,
    pub part_flg: u8,
    pub num_test: u16,
    pub hard_bin: u16,
    pub soft_bin: u16,
    pub x_coord: i16,
    pub y_coord: i16,
    pub test_t: u32,
    pub part_id: String,
    pub part_txt: String,
    pub part_fix: Vec<u8>,
}

impl Prr {
    /// PART_FLG bit 3 = part failed; bit 4 = no pass/fail indication.
    pub fn passed(&self) -> Option<bool> {
        if self.part_flg & 0x10 != 0 {
            None
        } else {
            Some(self.part_flg & 0x08 == 0)
        }
    }
    pub fn abnormal_end(&self) -> bool {
        self.part_flg & 0x04 != 0
    }
    pub fn is_retest(&self) -> bool {
        self.part_flg & 0x03 != 0
    }
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Tsr {
    pub head_num: u8,
    pub site_num: u8,
    pub test_typ: char,
    pub test_num: u32,
    pub exec_cnt: u32,
    pub fail_cnt: u32,
    pub alrm_cnt: u32,
    pub test_nam: String,
    pub seq_name: String,
    pub test_lbl: String,
    pub opt_flag: u8,
    pub test_tim: f32,
    pub test_min: f32,
    pub test_max: f32,
    pub tst_sums: f32,
    pub tst_sqrs: f32,
}

/// Parametric Test Record. Optional fields that were absent in the record
/// (truncated) are `None`.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Ptr {
    pub test_num: u32,
    pub head_num: u8,
    pub site_num: u8,
    pub test_flg: u8,
    pub parm_flg: u8,
    pub result: f32,
    pub test_txt: String,
    pub alarm_id: String,
    pub opt_flag: Option<u8>,
    pub res_scal: i8,
    pub llm_scal: i8,
    pub hlm_scal: i8,
    pub lo_limit: Option<f32>,
    pub hi_limit: Option<f32>,
    pub units: Option<String>,
    pub c_resfmt: String,
    pub c_llmfmt: String,
    pub c_hlmfmt: String,
    pub lo_spec: Option<f32>,
    pub hi_spec: Option<f32>,
}

impl Ptr {
    /// Result is usable as a number (TEST_FLG bits 1..5 clear).
    pub fn result_valid(&self) -> bool {
        self.test_flg & 0b0011_1110 == 0
    }
    /// Pass/fail as reported by the tester (None when flag invalid).
    pub fn passed(&self) -> Option<bool> {
        if self.test_flg & 0x40 != 0 {
            None
        } else {
            Some(self.test_flg & 0x80 == 0)
        }
    }
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Mpr {
    pub test_num: u32,
    pub head_num: u8,
    pub site_num: u8,
    pub test_flg: u8,
    pub parm_flg: u8,
    pub rtn_stat: Vec<u8>,
    pub rtn_rslt: Vec<f32>,
    pub test_txt: String,
    pub alarm_id: String,
    pub opt_flag: Option<u8>,
    pub res_scal: i8,
    pub llm_scal: i8,
    pub hlm_scal: i8,
    pub lo_limit: Option<f32>,
    pub hi_limit: Option<f32>,
    pub start_in: f32,
    pub incr_in: f32,
    pub rtn_indx: Vec<u16>,
    pub units: Option<String>,
    pub units_in: String,
    pub c_resfmt: String,
    pub c_llmfmt: String,
    pub c_hlmfmt: String,
    pub lo_spec: Option<f32>,
    pub hi_spec: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Ftr {
    pub test_num: u32,
    pub head_num: u8,
    pub site_num: u8,
    pub test_flg: u8,
    pub opt_flag: u8,
    pub cycl_cnt: u32,
    pub rel_vadr: u32,
    pub rept_cnt: u32,
    pub num_fail: u32,
    pub xfail_ad: i32,
    pub yfail_ad: i32,
    pub vect_off: i16,
    pub rtn_indx: Vec<u16>,
    pub rtn_stat: Vec<u8>,
    pub pgm_indx: Vec<u16>,
    pub pgm_stat: Vec<u8>,
    pub fail_pin: BitField,
    pub vect_nam: String,
    pub time_set: String,
    pub op_code: String,
    pub test_txt: String,
    pub alarm_id: String,
    pub prog_txt: String,
    pub rslt_txt: String,
    pub patg_num: u8,
    pub spin_map: BitField,
}

impl Ftr {
    pub fn passed(&self) -> Option<bool> {
        if self.test_flg & 0x40 != 0 {
            None
        } else {
            Some(self.test_flg & 0x80 == 0)
        }
    }
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Bps {
    pub seq_name: String,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Gdr {
    pub gen_data: Vec<GenData>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Dtr {
    pub text_dat: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "rec")]
pub enum Record {
    FAR(Far),
    ATR(Atr),
    MIR(Box<Mir>),
    MRR(Mrr),
    PCR(Pcr),
    HBR(Bin),
    SBR(Bin),
    PMR(Pmr),
    PGR(Pgr),
    PLR(Plr),
    RDR(Rdr),
    SDR(Box<Sdr>),
    WIR(Wir),
    WRR(Wrr),
    WCR(Wcr),
    PIR(Pir),
    PRR(Prr),
    TSR(Tsr),
    PTR(Ptr),
    MPR(Mpr),
    FTR(Box<Ftr>),
    BPS(Bps),
    EPS,
    GDR(Gdr),
    DTR(Dtr),
    Unknown { typ: u8, sub: u8, len: usize },
}

fn opt_r4(r: &mut FieldReader) -> Option<f32> {
    if r.remaining() >= 4 {
        Some(r.r4())
    } else {
        r.r4();
        None
    }
}

impl Record {
    pub fn name(&self) -> &'static str {
        let (t, s) = self.code();
        record_name(t, s)
    }

    pub fn code(&self) -> (u8, u8) {
        use code::*;
        match self {
            Record::FAR(_) => FAR,
            Record::ATR(_) => ATR,
            Record::MIR(_) => MIR,
            Record::MRR(_) => MRR,
            Record::PCR(_) => PCR,
            Record::HBR(_) => HBR,
            Record::SBR(_) => SBR,
            Record::PMR(_) => PMR,
            Record::PGR(_) => PGR,
            Record::PLR(_) => PLR,
            Record::RDR(_) => RDR,
            Record::SDR(_) => SDR,
            Record::WIR(_) => WIR,
            Record::WRR(_) => WRR,
            Record::WCR(_) => WCR,
            Record::PIR(_) => PIR,
            Record::PRR(_) => PRR,
            Record::TSR(_) => TSR,
            Record::PTR(_) => PTR,
            Record::MPR(_) => MPR,
            Record::FTR(_) => FTR,
            Record::BPS(_) => BPS,
            Record::EPS => EPS,
            Record::GDR(_) => GDR,
            Record::DTR(_) => DTR,
            Record::Unknown { typ, sub, .. } => (*typ, *sub),
        }
    }

    /// Decode a record payload.
    pub fn parse(typ: u8, sub: u8, data: &[u8], endian: Endian) -> Record {
        let mut r = FieldReader::new(data, endian);
        let r = &mut r;
        match (typ, sub) {
            (0, 10) => Record::FAR(Far {
                cpu_type: r.u1(),
                stdf_ver: r.u1(),
            }),
            (0, 20) => Record::ATR(Atr {
                mod_tim: r.u4(),
                cmd_line: r.cn(),
            }),
            (1, 10) => Record::MIR(Box::new(Mir {
                setup_t: r.u4(),
                start_t: r.u4(),
                stat_num: r.u1(),
                mode_cod: r.c1(),
                rtst_cod: r.c1(),
                prot_cod: r.c1(),
                burn_tim: r.u2(),
                cmod_cod: r.c1(),
                lot_id: r.cn(),
                part_typ: r.cn(),
                node_nam: r.cn(),
                tstr_typ: r.cn(),
                job_nam: r.cn(),
                job_rev: r.cn(),
                sblot_id: r.cn(),
                oper_nam: r.cn(),
                exec_typ: r.cn(),
                exec_ver: r.cn(),
                test_cod: r.cn(),
                tst_temp: r.cn(),
                user_txt: r.cn(),
                aux_file: r.cn(),
                pkg_typ: r.cn(),
                famly_id: r.cn(),
                date_cod: r.cn(),
                facil_id: r.cn(),
                floor_id: r.cn(),
                proc_id: r.cn(),
                oper_frq: r.cn(),
                spec_nam: r.cn(),
                spec_ver: r.cn(),
                flow_id: r.cn(),
                setup_id: r.cn(),
                dsgn_rev: r.cn(),
                eng_id: r.cn(),
                rom_cod: r.cn(),
                serl_num: r.cn(),
                supr_nam: r.cn(),
            })),
            (1, 20) => Record::MRR(Mrr {
                finish_t: r.u4(),
                disp_cod: r.c1(),
                usr_desc: r.cn(),
                exc_desc: r.cn(),
            }),
            (1, 30) => Record::PCR(Pcr {
                head_num: r.u1(),
                site_num: r.u1(),
                part_cnt: r.u4(),
                rtst_cnt: r.u4(),
                abrt_cnt: r.u4(),
                good_cnt: r.u4(),
                func_cnt: r.u4(),
            }),
            (1, 40) | (1, 50) => {
                let b = Bin {
                    head_num: r.u1(),
                    site_num: r.u1(),
                    bin_num: r.u2(),
                    bin_cnt: r.u4(),
                    bin_pf: r.c1(),
                    bin_nam: r.cn(),
                };
                if sub == 40 {
                    Record::HBR(b)
                } else {
                    Record::SBR(b)
                }
            }
            (1, 60) => Record::PMR(Pmr {
                pmr_indx: r.u2(),
                chan_typ: r.u2(),
                chan_nam: r.cn(),
                phy_nam: r.cn(),
                log_nam: r.cn(),
                head_num: r.u1(),
                site_num: r.u1(),
            }),
            (1, 62) => {
                let grp_indx = r.u2();
                let grp_nam = r.cn();
                let k = r.u2() as usize;
                Record::PGR(Pgr {
                    grp_indx,
                    grp_nam,
                    pmr_indx: r.vec_u2(k),
                })
            }
            (1, 63) => {
                let k = r.u2() as usize;
                Record::PLR(Plr {
                    grp_indx: r.vec_u2(k),
                    grp_mode: r.vec_u2(k),
                    grp_radx: r.vec_u1(k),
                    pgm_char: r.vec_cn(k),
                    rtn_char: r.vec_cn(k),
                    pgm_chal: r.vec_cn(k),
                    rtn_chal: r.vec_cn(k),
                })
            }
            (1, 70) => {
                let k = r.u2() as usize;
                Record::RDR(Rdr {
                    rtst_bin: r.vec_u2(k),
                })
            }
            (1, 80) => {
                let head_num = r.u1();
                let site_grp = r.u1();
                let k = r.u1() as usize;
                Record::SDR(Box::new(Sdr {
                    head_num,
                    site_grp,
                    site_num: r.vec_u1(k),
                    hand_typ: r.cn(),
                    hand_id: r.cn(),
                    card_typ: r.cn(),
                    card_id: r.cn(),
                    load_typ: r.cn(),
                    load_id: r.cn(),
                    dib_typ: r.cn(),
                    dib_id: r.cn(),
                    cabl_typ: r.cn(),
                    cabl_id: r.cn(),
                    cont_typ: r.cn(),
                    cont_id: r.cn(),
                    lasr_typ: r.cn(),
                    lasr_id: r.cn(),
                    extr_typ: r.cn(),
                    extr_id: r.cn(),
                }))
            }
            (2, 10) => Record::WIR(Wir {
                head_num: r.u1(),
                site_grp: r.u1(),
                start_t: r.u4(),
                wafer_id: r.cn(),
            }),
            (2, 20) => Record::WRR(Wrr {
                head_num: r.u1(),
                site_grp: r.u1(),
                finish_t: r.u4(),
                part_cnt: r.u4(),
                rtst_cnt: r.u4(),
                abrt_cnt: r.u4(),
                good_cnt: r.u4(),
                func_cnt: r.u4(),
                wafer_id: r.cn(),
                fabwf_id: r.cn(),
                frame_id: r.cn(),
                mask_id: r.cn(),
                usr_desc: r.cn(),
                exc_desc: r.cn(),
            }),
            (2, 30) => Record::WCR(Wcr {
                wafr_siz: r.r4(),
                die_ht: r.r4(),
                die_wid: r.r4(),
                wf_units: r.u1(),
                wf_flat: r.c1(),
                center_x: r.i2(),
                center_y: r.i2(),
                pos_x: r.c1(),
                pos_y: r.c1(),
            }),
            (5, 10) => Record::PIR(Pir {
                head_num: r.u1(),
                site_num: r.u1(),
            }),
            (5, 20) => Record::PRR(Prr {
                head_num: r.u1(),
                site_num: r.u1(),
                part_flg: r.b1(),
                num_test: r.u2(),
                hard_bin: r.u2(),
                soft_bin: r.u2(),
                x_coord: if r.remaining() >= 2 { r.i2() } else { -32768 },
                y_coord: if r.remaining() >= 2 { r.i2() } else { -32768 },
                test_t: r.u4(),
                part_id: r.cn(),
                part_txt: r.cn(),
                part_fix: r.bn(),
            }),
            (10, 30) => Record::TSR(Tsr {
                head_num: r.u1(),
                site_num: r.u1(),
                test_typ: r.c1(),
                test_num: r.u4(),
                exec_cnt: r.u4(),
                fail_cnt: r.u4(),
                alrm_cnt: r.u4(),
                test_nam: r.cn(),
                seq_name: r.cn(),
                test_lbl: r.cn(),
                opt_flag: r.b1(),
                test_tim: r.r4(),
                test_min: r.r4(),
                test_max: r.r4(),
                tst_sums: r.r4(),
                tst_sqrs: r.r4(),
            }),
            (15, 10) => Record::PTR(parse_ptr(r)),
            (15, 15) => Record::MPR(parse_mpr(r)),
            (15, 20) => Record::FTR(Box::new(parse_ftr(r))),
            (20, 10) => Record::BPS(Bps { seq_name: r.cn() }),
            (20, 20) => Record::EPS,
            (50, 10) => {
                let k = r.u2() as usize;
                Record::GDR(Gdr { gen_data: r.vn(k) })
            }
            (50, 30) => Record::DTR(Dtr { text_dat: r.cn() }),
            _ => Record::Unknown {
                typ,
                sub,
                len: data.len(),
            },
        }
    }

    /// Encode into a complete record (header + payload).
    pub fn encode(&self, endian: Endian) -> Vec<u8> {
        let mut w = FieldWriter::new(endian);
        let (t, s) = self.code();
        match self {
            Record::FAR(x) => {
                w.u1(x.cpu_type).u1(x.stdf_ver);
            }
            Record::ATR(x) => {
                w.u4(x.mod_tim).cn(&x.cmd_line);
            }
            Record::MIR(x) => {
                w.u4(x.setup_t)
                    .u4(x.start_t)
                    .u1(x.stat_num)
                    .c1(x.mode_cod)
                    .c1(x.rtst_cod)
                    .c1(x.prot_cod);
                w.u2(x.burn_tim).c1(x.cmod_cod);
                for f in [
                    &x.lot_id,
                    &x.part_typ,
                    &x.node_nam,
                    &x.tstr_typ,
                    &x.job_nam,
                    &x.job_rev,
                    &x.sblot_id,
                    &x.oper_nam,
                    &x.exec_typ,
                    &x.exec_ver,
                    &x.test_cod,
                    &x.tst_temp,
                    &x.user_txt,
                    &x.aux_file,
                    &x.pkg_typ,
                    &x.famly_id,
                    &x.date_cod,
                    &x.facil_id,
                    &x.floor_id,
                    &x.proc_id,
                    &x.oper_frq,
                    &x.spec_nam,
                    &x.spec_ver,
                    &x.flow_id,
                    &x.setup_id,
                    &x.dsgn_rev,
                    &x.eng_id,
                    &x.rom_cod,
                    &x.serl_num,
                    &x.supr_nam,
                ] {
                    w.cn(f);
                }
            }
            Record::MRR(x) => {
                w.u4(x.finish_t)
                    .c1(x.disp_cod)
                    .cn(&x.usr_desc)
                    .cn(&x.exc_desc);
            }
            Record::PCR(x) => {
                w.u1(x.head_num)
                    .u1(x.site_num)
                    .u4(x.part_cnt)
                    .u4(x.rtst_cnt)
                    .u4(x.abrt_cnt)
                    .u4(x.good_cnt)
                    .u4(x.func_cnt);
            }
            Record::HBR(x) | Record::SBR(x) => {
                w.u1(x.head_num)
                    .u1(x.site_num)
                    .u2(x.bin_num)
                    .u4(x.bin_cnt)
                    .c1(x.bin_pf)
                    .cn(&x.bin_nam);
            }
            Record::PMR(x) => {
                w.u2(x.pmr_indx)
                    .u2(x.chan_typ)
                    .cn(&x.chan_nam)
                    .cn(&x.phy_nam)
                    .cn(&x.log_nam)
                    .u1(x.head_num)
                    .u1(x.site_num);
            }
            Record::PGR(x) => {
                w.u2(x.grp_indx).cn(&x.grp_nam).u2(x.pmr_indx.len() as u16);
                for &i in &x.pmr_indx {
                    w.u2(i);
                }
            }
            Record::PLR(x) => {
                w.u2(x.grp_indx.len() as u16);
                x.grp_indx.iter().for_each(|&v| {
                    w.u2(v);
                });
                x.grp_mode.iter().for_each(|&v| {
                    w.u2(v);
                });
                x.grp_radx.iter().for_each(|&v| {
                    w.u1(v);
                });
                for v in [&x.pgm_char, &x.rtn_char, &x.pgm_chal, &x.rtn_chal] {
                    v.iter().for_each(|s| {
                        w.cn(s);
                    });
                }
            }
            Record::RDR(x) => {
                w.u2(x.rtst_bin.len() as u16);
                x.rtst_bin.iter().for_each(|&v| {
                    w.u2(v);
                });
            }
            Record::SDR(x) => {
                w.u1(x.head_num).u1(x.site_grp).u1(x.site_num.len() as u8);
                x.site_num.iter().for_each(|&v| {
                    w.u1(v);
                });
                for f in [
                    &x.hand_typ,
                    &x.hand_id,
                    &x.card_typ,
                    &x.card_id,
                    &x.load_typ,
                    &x.load_id,
                    &x.dib_typ,
                    &x.dib_id,
                    &x.cabl_typ,
                    &x.cabl_id,
                    &x.cont_typ,
                    &x.cont_id,
                    &x.lasr_typ,
                    &x.lasr_id,
                    &x.extr_typ,
                    &x.extr_id,
                ] {
                    w.cn(f);
                }
            }
            Record::WIR(x) => {
                w.u1(x.head_num)
                    .u1(x.site_grp)
                    .u4(x.start_t)
                    .cn(&x.wafer_id);
            }
            Record::WRR(x) => {
                w.u1(x.head_num)
                    .u1(x.site_grp)
                    .u4(x.finish_t)
                    .u4(x.part_cnt)
                    .u4(x.rtst_cnt)
                    .u4(x.abrt_cnt);
                w.u4(x.good_cnt)
                    .u4(x.func_cnt)
                    .cn(&x.wafer_id)
                    .cn(&x.fabwf_id)
                    .cn(&x.frame_id)
                    .cn(&x.mask_id);
                w.cn(&x.usr_desc).cn(&x.exc_desc);
            }
            Record::WCR(x) => {
                w.r4(x.wafr_siz)
                    .r4(x.die_ht)
                    .r4(x.die_wid)
                    .u1(x.wf_units)
                    .c1(x.wf_flat)
                    .i2(x.center_x)
                    .i2(x.center_y);
                w.c1(x.pos_x).c1(x.pos_y);
            }
            Record::PIR(x) => {
                w.u1(x.head_num).u1(x.site_num);
            }
            Record::PRR(x) => {
                w.u1(x.head_num)
                    .u1(x.site_num)
                    .u1(x.part_flg)
                    .u2(x.num_test)
                    .u2(x.hard_bin)
                    .u2(x.soft_bin);
                w.i2(x.x_coord)
                    .i2(x.y_coord)
                    .u4(x.test_t)
                    .cn(&x.part_id)
                    .cn(&x.part_txt)
                    .bn(&x.part_fix);
            }
            Record::TSR(x) => {
                w.u1(x.head_num)
                    .u1(x.site_num)
                    .c1(x.test_typ)
                    .u4(x.test_num)
                    .u4(x.exec_cnt)
                    .u4(x.fail_cnt);
                w.u4(x.alrm_cnt)
                    .cn(&x.test_nam)
                    .cn(&x.seq_name)
                    .cn(&x.test_lbl)
                    .u1(x.opt_flag);
                w.r4(x.test_tim)
                    .r4(x.test_min)
                    .r4(x.test_max)
                    .r4(x.tst_sums)
                    .r4(x.tst_sqrs);
            }
            Record::PTR(x) => {
                w.u4(x.test_num)
                    .u1(x.head_num)
                    .u1(x.site_num)
                    .u1(x.test_flg)
                    .u1(x.parm_flg)
                    .r4(x.result);
                w.cn(&x.test_txt).cn(&x.alarm_id);
                if let Some(opt) = x.opt_flag {
                    w.u1(opt).i1(x.res_scal).i1(x.llm_scal).i1(x.hlm_scal);
                    w.r4(x.lo_limit.unwrap_or(0.0))
                        .r4(x.hi_limit.unwrap_or(0.0));
                    w.cn(x.units.as_deref().unwrap_or(""))
                        .cn(&x.c_resfmt)
                        .cn(&x.c_llmfmt)
                        .cn(&x.c_hlmfmt);
                    w.r4(x.lo_spec.unwrap_or(0.0)).r4(x.hi_spec.unwrap_or(0.0));
                }
            }
            Record::MPR(x) => {
                w.u4(x.test_num)
                    .u1(x.head_num)
                    .u1(x.site_num)
                    .u1(x.test_flg)
                    .u1(x.parm_flg);
                w.u2(x.rtn_stat.len() as u16).u2(x.rtn_rslt.len() as u16);
                w.n1s(&x.rtn_stat);
                x.rtn_rslt.iter().for_each(|&v| {
                    w.r4(v);
                });
                w.cn(&x.test_txt).cn(&x.alarm_id);
                if let Some(opt) = x.opt_flag {
                    w.u1(opt).i1(x.res_scal).i1(x.llm_scal).i1(x.hlm_scal);
                    w.r4(x.lo_limit.unwrap_or(0.0))
                        .r4(x.hi_limit.unwrap_or(0.0))
                        .r4(x.start_in)
                        .r4(x.incr_in);
                    x.rtn_indx.iter().for_each(|&v| {
                        w.u2(v);
                    });
                    w.cn(x.units.as_deref().unwrap_or("")).cn(&x.units_in);
                    w.cn(&x.c_resfmt).cn(&x.c_llmfmt).cn(&x.c_hlmfmt);
                    w.r4(x.lo_spec.unwrap_or(0.0)).r4(x.hi_spec.unwrap_or(0.0));
                }
            }
            Record::FTR(x) => {
                w.u4(x.test_num)
                    .u1(x.head_num)
                    .u1(x.site_num)
                    .u1(x.test_flg)
                    .u1(x.opt_flag);
                w.u4(x.cycl_cnt)
                    .u4(x.rel_vadr)
                    .u4(x.rept_cnt)
                    .u4(x.num_fail)
                    .i4(x.xfail_ad)
                    .i4(x.yfail_ad);
                w.i2(x.vect_off)
                    .u2(x.rtn_indx.len() as u16)
                    .u2(x.pgm_indx.len() as u16);
                x.rtn_indx.iter().for_each(|&v| {
                    w.u2(v);
                });
                w.n1s(&x.rtn_stat);
                x.pgm_indx.iter().for_each(|&v| {
                    w.u2(v);
                });
                w.n1s(&x.pgm_stat);
                w.dn(&x.fail_pin)
                    .cn(&x.vect_nam)
                    .cn(&x.time_set)
                    .cn(&x.op_code)
                    .cn(&x.test_txt);
                w.cn(&x.alarm_id)
                    .cn(&x.prog_txt)
                    .cn(&x.rslt_txt)
                    .u1(x.patg_num)
                    .dn(&x.spin_map);
            }
            Record::BPS(x) => {
                w.cn(&x.seq_name);
            }
            Record::EPS => {}
            Record::GDR(x) => {
                w.u2(x.gen_data.len() as u16);
                for g in &x.gen_data {
                    match g {
                        GenData::Pad => w.u1(0),
                        GenData::U1(v) => w.u1(1).u1(*v),
                        GenData::U2(v) => w.u1(2).u2(*v),
                        GenData::U4(v) => w.u1(3).u4(*v),
                        GenData::I1(v) => w.u1(4).i1(*v),
                        GenData::I2(v) => w.u1(5).i2(*v),
                        GenData::I4(v) => w.u1(6).i4(*v),
                        GenData::R4(v) => w.u1(7).r4(*v),
                        GenData::R8(v) => w.u1(8).r8(*v),
                        GenData::Cn(v) => w.u1(10).cn(v),
                        GenData::Bn(v) => w.u1(11).bn(v),
                        GenData::Dn(v) => w.u1(12).dn(v),
                        GenData::N1(v) => w.u1(13).u1(*v),
                    };
                }
            }
            Record::DTR(x) => {
                w.cn(&x.text_dat);
            }
            Record::Unknown { .. } => {}
        }
        w.finish(t, s)
    }
}

fn parse_ptr(r: &mut FieldReader) -> Ptr {
    let mut p = Ptr {
        test_num: r.u4(),
        head_num: r.u1(),
        site_num: r.u1(),
        test_flg: r.b1(),
        parm_flg: r.b1(),
        result: r.r4(),
        test_txt: r.cn(),
        alarm_id: r.cn(),
        ..Default::default()
    };
    if r.is_empty() {
        return p;
    }
    p.opt_flag = Some(r.b1());
    p.res_scal = r.i1();
    p.llm_scal = r.i1();
    p.hlm_scal = r.i1();
    p.lo_limit = opt_r4(r);
    p.hi_limit = opt_r4(r);
    if !r.is_empty() {
        p.units = Some(r.cn());
    }
    p.c_resfmt = r.cn();
    p.c_llmfmt = r.cn();
    p.c_hlmfmt = r.cn();
    p.lo_spec = opt_r4(r);
    p.hi_spec = opt_r4(r);
    p
}

fn parse_mpr(r: &mut FieldReader) -> Mpr {
    let mut m = Mpr {
        test_num: r.u4(),
        head_num: r.u1(),
        site_num: r.u1(),
        test_flg: r.b1(),
        parm_flg: r.b1(),
        ..Default::default()
    };
    let j = r.u2() as usize;
    let k = r.u2() as usize;
    m.rtn_stat = r.vec_n1(j);
    m.rtn_rslt = r.vec_r4(k);
    m.test_txt = r.cn();
    m.alarm_id = r.cn();
    if r.is_empty() {
        return m;
    }
    m.opt_flag = Some(r.b1());
    m.res_scal = r.i1();
    m.llm_scal = r.i1();
    m.hlm_scal = r.i1();
    m.lo_limit = opt_r4(r);
    m.hi_limit = opt_r4(r);
    m.start_in = r.r4();
    m.incr_in = r.r4();
    m.rtn_indx = r.vec_u2(j);
    if !r.is_empty() {
        m.units = Some(r.cn());
    }
    m.units_in = r.cn();
    m.c_resfmt = r.cn();
    m.c_llmfmt = r.cn();
    m.c_hlmfmt = r.cn();
    m.lo_spec = opt_r4(r);
    m.hi_spec = opt_r4(r);
    m
}

fn parse_ftr(r: &mut FieldReader) -> Ftr {
    let mut f = Ftr {
        test_num: r.u4(),
        head_num: r.u1(),
        site_num: r.u1(),
        test_flg: r.b1(),
        ..Default::default()
    };
    if r.is_empty() {
        return f;
    }
    f.opt_flag = r.b1();
    f.cycl_cnt = r.u4();
    f.rel_vadr = r.u4();
    f.rept_cnt = r.u4();
    f.num_fail = r.u4();
    f.xfail_ad = r.i4();
    f.yfail_ad = r.i4();
    f.vect_off = r.i2();
    let j = r.u2() as usize;
    let k = r.u2() as usize;
    f.rtn_indx = r.vec_u2(j);
    f.rtn_stat = r.vec_n1(j);
    f.pgm_indx = r.vec_u2(k);
    f.pgm_stat = r.vec_n1(k);
    f.fail_pin = r.dn();
    f.vect_nam = r.cn();
    f.time_set = r.cn();
    f.op_code = r.cn();
    f.test_txt = r.cn();
    f.alarm_id = r.cn();
    f.prog_txt = r.cn();
    f.rslt_txt = r.cn();
    f.patg_num = r.u1();
    f.spin_map = r.dn();
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rt(rec: Record) {
        for e in [Endian::Little, Endian::Big] {
            let bytes = rec.encode(e);
            let (t, s) = (bytes[2], bytes[3]);
            let back = Record::parse(t, s, &bytes[4..], e);
            assert_eq!(back, rec, "roundtrip {}", rec.name());
        }
    }

    #[test]
    fn roundtrips() {
        rt(Record::FAR(Far {
            cpu_type: 2,
            stdf_ver: 4,
        }));
        rt(Record::MIR(Box::new(Mir {
            lot_id: "LOT1".into(),
            mode_cod: 'P',
            rtst_cod: ' ',
            prot_cod: ' ',
            cmod_cod: ' ',
            ..Default::default()
        })));
        rt(Record::PRR(Prr {
            head_num: 1,
            site_num: 3,
            part_flg: 8,
            hard_bin: 5,
            soft_bin: 50,
            x_coord: -3,
            y_coord: 7,
            part_id: "17".into(),
            ..Default::default()
        }));
        rt(Record::PTR(Ptr {
            test_num: 1000,
            head_num: 1,
            site_num: 2,
            result: 1.25,
            test_txt: "VDD_LEAK".into(),
            opt_flag: Some(0),
            lo_limit: Some(-1.0),
            hi_limit: Some(2.0),
            units: Some("uA".into()),
            lo_spec: Some(0.0),
            hi_spec: Some(0.0),
            ..Default::default()
        }));
        rt(Record::PTR(Ptr {
            test_num: 5,
            result: 3.0,
            ..Default::default()
        }));
        rt(Record::MPR(Mpr {
            test_num: 7,
            rtn_stat: vec![1, 2, 3],
            rtn_rslt: vec![0.5, 0.6, 0.7],
            opt_flag: Some(0),
            lo_limit: Some(0.0),
            hi_limit: Some(1.0),
            rtn_indx: vec![1, 2, 3],
            units: Some("V".into()),
            lo_spec: Some(0.0),
            hi_spec: Some(0.0),
            ..Default::default()
        }));
        rt(Record::GDR(Gdr {
            gen_data: vec![GenData::U2(3), GenData::Cn("x".into()), GenData::R8(2.5)],
        }));
    }

    #[test]
    fn prr_flags() {
        let p = Prr {
            part_flg: 0x08,
            ..Default::default()
        };
        assert_eq!(p.passed(), Some(false));
        let p = Prr {
            part_flg: 0x00,
            ..Default::default()
        };
        assert_eq!(p.passed(), Some(true));
        let p = Prr {
            part_flg: 0x10,
            ..Default::default()
        };
        assert_eq!(p.passed(), None);
    }
}
