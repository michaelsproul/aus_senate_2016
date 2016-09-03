extern crate aus_senate;
extern crate csv;
extern crate rustc_serialize;

use std::error::Error;
use std::io::Read;
use std::env;
use std::fs::File;

use aus_senate::group::*;
use aus_senate::candidate::*;
use aus_senate::ballot::*;
use aus_senate::voting::*;
use aus_senate::util::*;
use aus_senate::parse::*;

#[derive(RustcDecodable, Debug)]
struct CandidateRow {
    txn_nm: String,
    nom_ty: String,
    state_ab: String,
    div_nm: String,
    ticket: String,
    ballot_position: u32,
    surname: String,
    other_names: String,
    party: String,
    occupation: String,
    address_1: String,
    address_2: String,
    postcode: String,
    suburb: String,
    address_state: String,
    contact_work_ph: String,
    contact_home_ph: String,
    postal_address_1: String,
    postal_address_2: String,
    postal_suburb: String,
    postal_postcode: String,
    contact_fax: String,
    postal_state_ab: String,
    contact_mobile: String,
    contact_email: String,
}

fn parse_candidates_file<R: Read>(input: R) -> Result<Vec<Candidate>, Box<Error>> {
    let mut result = vec![];
    let mut reader = csv::Reader::from_reader(input);

    for (id, raw_row) in reader.decode::<CandidateRow>().enumerate() {
        let row = try!(raw_row);
        if row.nom_ty != "S" {
            continue;
        }
        result.push(Candidate {
            id: id as u32,
            surname: row.surname,
            other_names: row.other_names,
            group_name: row.ticket,
            party: row.party,
            state: row.state_ab,
        });
    }

    Ok(result)
}

fn get_candidate_id_list(candidates: &[Candidate], state: &String) -> Vec<CandidateId> {
    candidates.iter()
        .filter(|c| &c.state == state)
        .map(|c| c.id)
        .collect()
}

#[derive(RustcDecodable, Debug)]
struct PrefRow {
    electorate_name: String,
    vote_collection_point: String,
    vote_collection_point_id: String,
    batch_num: String,
    paper_num: String,
    preferences: String,
}

fn ballot_below_the_line(raw_prefs: Vec<&str>, candidates: &[CandidateId]) -> Result<Vec<CandidateId>, BallotParseErr> {
    let mut pref_map = HashMap::new();

    for (idx, pref) in raw_prefs.iter().enumerate() {
        if pref.is_empty() {
            continue;
        }
        let pref_int = try!(pref.parse::<u32>().map_err(|_| InvalidBallot("Pref not an int".to_string())));
        pref_map.insert(candidates[idx], pref_int);
    }
    let prefs = pref_map_to_vec(pref_map);
    Ok(prefs)
}

// FIXME: this is a bit of a mess.
fn ballot_above_the_line(raw_prefs: Vec<&str>, groups: &[Group]) -> Result<Vec<CandidateId>, BallotParseErr> {
    let mut pref_map = HashMap::new();

    for (group_idx, pref) in raw_prefs.iter().enumerate() {
        if pref.is_empty() {
            continue;
        }
        let pref_int = try!(pref.parse::<u32>().map_err(|_| InvalidBallot("Pref not an int".to_string())));
        pref_map.insert(pref_int, &groups[group_idx].candidate_ids);
    }

    let mut flat_pref_map: Vec<_> = pref_map.into_iter().collect();
    flat_pref_map.sort_by_key(|&(pref, _)| pref);
    let mut prefs = vec![];
    for (_, group_candidates) in flat_pref_map {
        prefs.extend_from_slice(group_candidates);
    }
    Ok(prefs)
}

// Convert a preferences string to a ballot.
// TODO: Extend ballot parsing.
fn pref_string_to_ballot(pref_string: &str, groups: &[Group], candidates: &[CandidateId])
    -> Result<Vec<CandidateId>, BallotParseErr>
{
    // Split the preference string into above and below the line sections.
    //println!("Pref string: {}", pref_string);
    //println!("Groups: {:?}", groups);
    let mut above_the_line: Vec<&str> = pref_string.split(',').collect();
    let below_the_line = above_the_line.split_off(groups.len());

    // A preference is valid if any of the comma separated values are non-empty.
    let is_valid = |prefs: &[&str]| prefs.iter().any(|s| !s.is_empty());

    match (is_valid(&above_the_line), is_valid(&below_the_line)) {
        (true, false) => ballot_above_the_line(above_the_line, groups),
        (false, true) => ballot_below_the_line(below_the_line, candidates),
        (true, true) => Err(InvalidBallot("both valid".to_string())),
        (false, false) => Err(InvalidBallot("neither valid".to_string())),
    }
}

fn parse_single_ballot(raw_row: csv::Result<PrefRow>, groups: &[Group], candidates: &[CandidateId]) -> IOBallot {
    match raw_row {
        Ok(row) => {
            pref_string_to_ballot(&row.preferences, groups, candidates).map(|prefs| {
                MultiBallot::single(prefs)
            })
        }
        Err(e) => Err(InputError(From::from(e)))
    }
}

// FIXME: This macro is to avoid writing the iterator type. Can use a function once impl Trait lands.
macro_rules! parse_preferences_file {
    ($reader:expr, $groups:expr, $candidates:expr) => {
        $reader
            .decode::<PrefRow>()
            .map(|raw_row| parse_single_ballot(raw_row, $groups, $candidates))
    }
}

fn main_with_result() -> Result<(), Box<Error>> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 4 && args.len() != 5 {
        println!("Usage: ./election2016 <candidates file> <prefs file> <state> [num candidates]");
        try!(Err("invalid command line arguments.".to_string()));
    }

    let candidates_file_name = &args[1];
    let prefs_file_name = &args[2];
    let state = &args[3];
    let num_candidates = match args.get(4) {
        Some(x) => try!(x.parse::<u32>()),
        None => 12
    };

    let candidates_file = try!(File::open(candidates_file_name));
    let all_candidates = try!(parse_candidates_file(candidates_file));

    for c in all_candidates.iter() {
        println!("{}: {} {} ({})", c.id, c.other_names, c.surname, c.party);
    }

    // Extract candidate and group information from the complete list of ballots.
    let candidates = get_state_candidates(&all_candidates, state);
    // FIXME: kill candidate_ids.
    let candidate_ids = get_candidate_id_list(&all_candidates, state);
    let groups = get_group_list(&all_candidates, state);

    println!("Num groups: {}", groups.len());
    println!("Groups: {:#?}", groups);

    let prefs_file = try!(open_aec_csv(prefs_file_name));

    let mut csv_reader = csv::Reader::from_reader(prefs_file);
    let ballots_iter = parse_preferences_file!(csv_reader, &groups, &candidate_ids);

    let election_result = try!(decide_election(&candidates, ballots_iter, num_candidates));

    for c in election_result.senators.iter() {
        println!("Elected: {} {} ({})", c.other_names, c.surname, c.party);
    }

    if election_result.tied {
        println!("Tie for the last place");
    }

    Ok(())
}

fn main() {
    if let Err(e) = main_with_result() {
        println!("Error: {:?}", e);
    }
}
